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
//! # Attachments
//!
//! The text of an attachment (`attachment.text`) is read in the run's
//! task, not before `ai_run` answers: reading a long PDF takes a moment,
//! and the commands of a connection run one after the other, so the
//! client's other requests would wait behind it. A file that cannot be
//! read then fails the run (`AiFailed`, `ai_bad_input`) instead of
//! refusing `ai_run`; nothing was sent. The row first holds the system
//! prompt alone, and the whole message once read (`ai_run_set_input`).
//!
//! # What is wired so far
//!
//! Every scope and every built-in prompt. Not yet: the inputs
//! `message.eml`, `draft.full` and `targets` outside a selection, the
//! outputs `insert` and `reply`; a user's prompt that needs one is listed
//! with `available: false` (`available`), and `ai_run` refuses it with a
//! clear message.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use nm_ai::answer::{self, Answer, Finding, Sent};
use nm_ai::message::Mail;
use nm_ai::catalog::{self, Loaded};
use nm_ai::prompt::{Input, On, Output, Prompt, Scope, Screen, Source as PromptSource};
use nm_ai::{AiError, AiRequest, Provider};
use nm_api::{
    AiActionView, AiApplyParams, AiAddressView, AiApplyResult, AiContactView, AiEventView, AiFile, AiPhoneView, AiDiffPiece, AiDismissSuggestionParams, AiSuggestionView, AiSuggestionsParams, AiListPromptsParams, AiPromptView, AiProposal, AiRunIdParams, AiRunParams,
    AiRunResult, AiRunView, AiSegmentChange, AiStatusParams, AiStatusResult, AiVariantView, CommandResult, DraftView,
    Event, MessageView, RpcError, error_code,
};
use nm_core::{FolderRole, Source};
use nm_engine::config::{AiProviderKind, AiSettings};
use nm_store::ai_runs::{Decision, NewRun, NewSuggestion, RunDone, RunEnd};
use nm_store::attachment_text::{Kind as FileKind, Unreadable};
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
    target: Target,
    /// What the core found itself in the mail, for the floor of a security
    /// verdict (`nm_ai::answer::apply_floor`).
    findings: Vec<Finding>,
}

/// What a run works on.
#[derive(Clone)]
enum Target {
    /// A draft, at a revision; its body then, and the byte ranges of the
    /// segments sent.
    Draft { draft_id: i64, revision: i64, body: String, ranges: Vec<Range<usize>> },
    /// A mail and its conversation; `message` is the mail read, which a
    /// calendar file or a contact card links back to
    /// (`nicemailer://m/<id>`); `sender` its From address, which a contact
    /// card holds whatever the model answered (`answer::Sent::sender`).
    Mail { message: i64, sender: Option<String> },
    /// Several conversations, sorted in batches.
    Selection,
    /// An attachment of the mail `message`.
    Attachment { message: i64 },
}

impl Target {
    fn segments(&self) -> usize {
        match self {
            Target::Draft { ranges, .. } => ranges.len(),
            Target::Mail { .. } | Target::Selection | Target::Attachment { .. } => 0,
        }
    }

    fn sender(&self) -> Option<&str> {
        match self {
            Target::Mail { sender, .. } => sender.as_deref(),
            Target::Draft { .. } | Target::Selection | Target::Attachment { .. } => None,
        }
    }
}

struct Running {
    abort: Option<tokio::task::AbortHandle>,
    context: Context,
}

struct Finished {
    context: Context,
    answer: Answer,
    /// For a run in batches: those whose answer could not be read.
    failed_batches: u64,
    /// For appointments: a calendar file was given, which accepted the run
    /// (`apply`); closing the panel then is no refusal.
    saved: bool,
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
    match p.scope {
        Scope::Draft => {
            p.inputs.iter().all(|i| {
                matches!(i, Input::DraftOwnText | Input::DraftContext | Input::DraftHeaders | Input::DraftReplied | Input::UserIdentity)
            })
                && match p.output {
                    Output::Replace | Output::Subjects => true,
                    // Other wordings: the one chosen replaces the
                    // passage selected.
                    Output::Choices => p.then == Some(nm_ai::prompt::Then::Replace),
                    _ => false,
                }
        }
        Scope::Message | Scope::Thread => {
            p.inputs.iter().all(|i| {
                matches!(
                    i,
                    Input::MessageText
                        | Input::MessageHeaders
                        | Input::MessageAuth
                        | Input::ThreadText
                        | Input::SenderHistory
                        | Input::UserIdentity
                )
            }) && match p.output {
                Output::Panel | Output::Summary | Output::Verdict | Output::Events | Output::Contact => true,
                Output::Choices => p.then == Some(nm_ai::prompt::Then::Reply),
                _ => false,
            }
        }
        Scope::Selection => {
            p.output == Output::Targets && p.inputs.iter().all(|i| matches!(i, Input::SelectionItems | Input::Targets))
        }
        Scope::Attachment => {
            p.inputs.iter().all(|i| matches!(i, Input::AttachmentText | Input::UserIdentity))
                && matches!(p.output, Output::Panel | Output::Summary)
        }
    }
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
    let Some(screen) = Screen::parse(&p.screen) else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, format!("unknown screen {:?}", p.screen)));
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
            unmarked: match q.unmarked {
                nm_ai::prompt::Unmarked::Cursor => "cursor".into(),
                nm_ai::prompt::Unmarked::Folder => "folder".into(),
            },
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
    if prompt.scope == Scope::Selection {
        return run_selection(core, &s, prompt, variant, system, &p, source).await;
    }
    let (message, target, target_json, findings, deferred) = match prompt.scope {
        Scope::Draft => with_nothing_deferred(draft_inputs(core, &prompt, &p).await?),
        Scope::Attachment => {
            let (target, json, deferred) = attachment_inputs(core, &prompt, &p).await?;
            // The message is made once the file is read (module
            // documentation, "Attachments").
            (String::new(), target, json, Vec::new(), Some(deferred))
        }
        _ => with_nothing_deferred(mail_inputs(core, &prompt, &p).await?),
    };
    let request = AiRequest {
        system,
        message,
        model: prompt.model.clone().unwrap_or_else(|| s.model.clone()),
        schema: prompt.output.schema(&[]),
        max_output_tokens: prompt.max_output_tokens,
    };
    let provider = provider(core, &s).map_err(ai_error)?;

    let input = match &deferred {
        None => format!("{}\n\n{}", request.system, request.message),
        Some(d) => format!("{}\n\n[the text of {} is read next]", request.system, d.name),
    };
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
            target: &target_json,
            lang: &row.7,
            input: &input,
        })
    })
    .await?;

    let streamed = !prompt.output.is_structured();
    let context = Context { prompt, target, findings };
    core.ai.running.lock().unwrap().insert(run_id, Running { abort: None, context });
    let task = tokio::spawn(execute(core.clone(), run_id, request, provider, deferred));
    // The task may already have ended (a fake answer): then it is no longer
    // in the map, and there is nothing left to abort.
    if let Some(r) = core.ai.running.lock().unwrap().get_mut(&run_id) {
        r.abort = Some(task.abort_handle());
    }
    Ok(AiRunResult { run_id, streamed })
}

type Inputs = (String, Target, String, Vec<Finding>);

fn with_nothing_deferred((message, target, json, findings): Inputs) -> (String, Target, String, Vec<Finding>, Option<Deferred>) {
    (message, target, json, findings, None)
}

/// An attachment whose text the run's task reads before calling the
/// provider (module documentation, "Attachments").
struct Deferred {
    path: PathBuf,
    kind: FileKind,
    /// The file's name, as the user sees it.
    name: String,
    /// The sections of the message in the prompt's order, `None` where the
    /// attachment's text goes.
    sections: Vec<Option<String>>,
}

/// The inputs of an attachment prompt: the file is extracted from the raw
/// mail now (its path is all the task needs), its text read later.
/// Refused at once: an attachment unknown, not downloaded, or of a kind
/// whose text cannot be read (an image, an archive).
async fn attachment_inputs(core: &Arc<Core>, prompt: &Prompt, p: &AiRunParams) -> Result<(Target, String, Deferred), RpcError> {
    let Some(id) = p.attachment_id else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "this prompt needs attachment_id"));
    };
    let (raw, files) = (core.raw.clone(), core.attachments.clone());
    let declared = crate::drafts::declared_addresses(core);
    let read = with_store(core, move |st| {
        let file = st.get_attachment(&raw, &files, id)?;
        let (message, thread): (i64, Option<i64>) = st.conn().query_row(
            "SELECT m.id, m.thread_id FROM attachment a JOIN message m ON m.id = a.message_id WHERE a.id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok((file, message, thread, own_addresses(st, declared)?))
    })
    .await;
    let (file, message, thread, own) = crate::sync::repaired(core, read).await?;
    let Some(kind) = FileKind::of(&file.mime_type, Some(&file.filename)) else {
        return Err(ai_error(AiError::BadInput(format!("{}: {}", file.filename, Unreadable::Unsupported))));
    };
    let sections = prompt
        .inputs
        .iter()
        .map(|input| match input {
            Input::AttachmentText => None,
            Input::UserIdentity => Some(nm_ai::message::identity(&own)),
            _ => unreachable!("`available` lets only these inputs through"),
        })
        .collect();
    let json = serde_json::json!({ "attachment": id, "message": message, "thread": thread }).to_string();
    Ok((Target::Attachment { message }, json, Deferred { path: file.path, kind, name: file.filename, sections }))
}

/// The inputs of a draft prompt (`draft.own_text`, `draft.headers`): the
/// message, the target, its JSON for the history, no findings.
async fn draft_inputs(
    core: &Arc<Core>,
    prompt: &Prompt,
    p: &AiRunParams,
) -> Result<(String, Target, String, Vec<Finding>), RpcError> {
    let (Some(draft_id), Some(revision)) = (p.draft_id, p.revision) else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "this prompt needs draft_id and revision"));
    };
    let draft = with_reader(core, move |st| st.get_draft(draft_id)).await?;
    if draft.revision != revision {
        let mut err = RpcError::new(error_code::REVISION_CONFLICT, "the draft changed since: save it and run again");
        err.data = Some(serde_json::json!({ "revision": draft.revision }));
        return Err(err);
    }
    // A prompt of a passage needs one, a prompt of the whole text takes
    // none (`on`, `ia.md`, section 4): the palette offers each only when
    // it fits, so another client is told what is wrong.
    match (prompt.on, p.selection.is_some()) {
        (On::Selection, false) => {
            return Err(ai_error(AiError::BadInput("select the text to work on first".into())));
        }
        (On::Text, true) => {
            return Err(ai_error(AiError::BadInput("this prompt works on the whole text: send no selection".into())));
        }
        _ => {}
    }
    // A formatted draft holds HTML: the AI reads and rewrites text, and
    // splicing its answer into the HTML would break it. Refused until the
    // AI works on formatted text (`edition.md`, section 14).
    if draft.body_format == "html" {
        return Err(ai_error(AiError::BadInput("the AI works on text only for now: switch the draft to text (Shift-Cmd-T)".into())));
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
            // The passage alone, then the draft around it (`DraftContext`).
            Input::DraftOwnText if prompt.on == On::Selection => {
                nm_ai::message::passage(texts.first().copied().unwrap_or("")).map_err(ai_error)?
            }
            Input::DraftOwnText => nm_ai::message::draft_text(&texts).map_err(ai_error)?,
            // `on = "selection"` (the catalog checks it): one range.
            Input::DraftContext => nm_ai::message::draft_context(&body, ranges[0].clone()),
            Input::DraftHeaders => draft_headers(&draft),
            Input::DraftReplied => {
                let replied = replied_mail(core, &draft).await?;
                nm_ai::message::replied(replied.as_ref())
            }
            // The draft's From is the user for this mail: the account it
            // is sent from, with the name it signs with.
            Input::UserIdentity => {
                let from = label(draft.from.name.as_deref(), Some(&draft.from.addr));
                nm_ai::message::draft_identity(&from, nm_core::segments::has_signature(&body))
            }
            _ => unreachable!("`available` lets only these inputs through"),
        });
    }
    let target_json = serde_json::json!({ "draft": draft_id }).to_string();
    Ok((sections.join("\n\n"), Target::Draft { draft_id, revision, body, ranges }, target_json, Vec::new()))
}

/// The mail a draft replies to, as the inputs see it (`draft.replied`);
/// `None` for a new mail or a forward, and for a reply whose mail is gone
/// or not downloaded yet: the model then writes from the notes alone,
/// rather than the run failing for a context it can do without.
async fn replied_mail(core: &Arc<Core>, draft: &DraftView) -> Result<Option<Mail>, RpcError> {
    let (Some(message_id), Some(thread_id)) = (draft.reply_to, draft.thread_id) else { return Ok(None) };
    let declared = crate::drafts::declared_addresses(core);
    let found = with_reader(core, move |st| {
        let Ok((_, messages)) = st.get_thread(thread_id, &declared) else { return Ok(None) };
        let own = own_addresses(st, declared)?;
        Ok(messages.into_iter().find(|m| m.id == message_id).map(|m| {
            let mine = m.from_addr.as_deref().is_some_and(|a| own.contains(&a.to_lowercase()));
            (m, mine)
        }))
    })
    .await?;
    Ok(found.filter(|(m, _)| m.body_text.is_some()).map(|(m, mine)| mail_of(&m, mine)))
}

/// The inputs of a mail prompt, from the conversation as the reader shows
/// it (`get_thread`): the mail is `message_id`, else the latest received
/// one (the latest of all when every one is the user's).
async fn mail_inputs(
    core: &Arc<Core>,
    prompt: &Prompt,
    p: &AiRunParams,
) -> Result<(String, Target, String, Vec<Finding>), RpcError> {
    let Some(thread_id) = p.thread_id else {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "this prompt needs thread_id"));
    };
    // The user's addresses: those of `accounts.toml`, and those of the
    // accounts in the database (an archive account imported from an mbox
    // is in the database only).
    let declared = crate::drafts::declared_addresses(core);
    let (messages, own) = with_reader(core, move |st| {
        let (_, messages) = st.get_thread(thread_id, &declared)?;
        Ok((messages, own_addresses(st, declared)?))
    })
    .await?;
    let is_mine = |m: &MessageView| m.from_addr.as_deref().is_some_and(|a| own.contains(&a.to_lowercase()));
    let message = match p.message_id {
        Some(id) => messages
            .iter()
            .find(|m| m.id == id)
            .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no message {id} in thread {thread_id}")))?,
        None => messages
            .iter()
            .rev()
            .find(|m| !is_mine(m))
            .or(messages.last())
            .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("thread {thread_id} has no message")))?,
    };
    if message.body_text.is_none() {
        return Err(ai_error(AiError::BadInput("this mail is still downloading: try again in a moment".into())));
    }
    let mail = mail_of(message, is_mine(message));
    let findings = findings(message);
    let mut sections = Vec::new();
    for input in &prompt.inputs {
        sections.push(match input {
            Input::MessageText => nm_ai::message::message_text(&mail),
            Input::MessageHeaders => nm_ai::message::message_headers(&mail),
            Input::MessageAuth => nm_ai::message::auth(&message.auth_result, &findings),
            Input::ThreadText => {
                let mails: Vec<Mail> =
                    messages.iter().filter(|m| m.body_text.is_some()).map(|m| mail_of(m, is_mine(m))).collect();
                nm_ai::message::thread_text(&mails)
            }
            Input::SenderHistory => {
                let addr = message.from_addr.clone().unwrap_or_default().to_lowercase();
                let lines = with_reader(core, move |st| sender_history(st, &addr)).await?;
                nm_ai::message::sender_history(&lines)
            }
            Input::UserIdentity => nm_ai::message::identity(&own),
            _ => unreachable!("`available` lets only these inputs through"),
        });
    }
    let target_json = serde_json::json!({ "thread": thread_id, "message": message.id }).to_string();
    Ok((sections.join("\n\n"), Target::Mail { message: message.id, sender: message.from_addr.clone() }, target_json, findings))
}

/// The user's addresses: those of `accounts.toml`, and those of the
/// accounts in the database (an archive account imported from an mbox is
/// in the database only).
fn own_addresses(st: &nm_store::Store, declared: Vec<String>) -> Result<Vec<String>, nm_store::StoreError> {
    let mut own = declared;
    let mut q = st.conn().prepare("SELECT lower(address) FROM account WHERE address IS NOT NULL")?;
    for a in q.query_map([], |r| r.get::<_, String>(0))? {
        let a = a?;
        if !own.contains(&a) {
            own.push(a);
        }
    }
    Ok(own)
}

/// A prompt of several conversations (`ia.md`, section 11, "Quarantine"):
/// the conversations and the targets are found now, the batches built and
/// sent in the background, one after the other, their suggestions kept as
/// each ends (`AiSuggestions`), the progress announced (`AiProgress`).
async fn run_selection(
    core: &Arc<Core>,
    s: &AiSettings,
    prompt: Prompt,
    variant: Option<char>,
    system: String,
    p: &AiRunParams,
    source: Source,
) -> Result<AiRunResult, RpcError> {
    let folders = p.folder_ids.clone().unwrap_or_default();
    if folders.is_empty() {
        return Err(RpcError::new(error_code::INVALID_PARAMS, "this prompt needs folder_ids, the folders shown"));
    }
    let marked = p.thread_ids.clone().unwrap_or_default();
    let whole_folder = marked.is_empty() && prompt.unmarked == nm_ai::prompt::Unmarked::Folder;
    let max = s.max_folder_items as usize;
    let (role, threads) = with_reader(core, move |st| {
        let mut role: Option<FolderRole> = None;
        let mut threads: Vec<(i64, i64)> = Vec::new();
        for f in &folders {
            let (r, account): (Option<String>, i64) =
                st.conn().query_row("SELECT role, account_id FROM folder WHERE id = ?1", [f], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let r = r.as_deref().and_then(FolderRole::parse);
            if role.is_some() && r != role {
                return Err(nm_store::StoreError::InvalidParams("the folders shown have different roles".into()));
            }
            role = r;
            let Some(r) = r else { continue };
            if whole_folder {
                // The newest first, as the list shows them; a conversation
                // in Quarantine is any with a mail there (`has_quarantine`).
                let sql = if r == FolderRole::Quarantine {
                    "SELECT id, date_last_received FROM thread WHERE account_id = ?1 AND has_quarantine = 1
                      ORDER BY date_last_received DESC LIMIT ?3"
                } else {
                    "SELECT id, date_last_received FROM thread WHERE account_id = ?1 AND folder = ?2
                      ORDER BY date_last_received DESC LIMIT ?3"
                };
                let mut q = st.conn().prepare(sql)?;
                let rows = q.query_map(nm_store::rusqlite::params![account, r.code(), max as i64], |row| Ok((row.get(0)?, row.get(1)?)))?;
                for row in rows {
                    threads.push(row?);
                }
            }
        }
        threads.sort_by_key(|t| std::cmp::Reverse(t.1));
        threads.truncate(max);
        Ok((role, threads.into_iter().map(|t| t.0).collect::<Vec<i64>>()))
    })
    .await?;
    let Some(role) = role else {
        return Err(ai_error(AiError::BadInput("this folder has no role: nothing to sort it into".into())));
    };
    if !prompt.offered_in(Some(role)) {
        return Err(ai_error(AiError::BadInput(format!("the prompt {} is not offered in this folder", prompt.id))));
    }
    let threads = if whole_folder { threads } else { marked };
    if threads.is_empty() {
        return Err(ai_error(AiError::BadInput(if whole_folder {
            "this folder is empty: nothing to sort".into()
        } else {
            "mark the conversations to sort, or put the cursor on one".into()
        })));
    }
    let targets = role.move_targets();
    let provider = provider(core, s).map_err(ai_error)?;
    let model = prompt.model.clone().unwrap_or_else(|| s.model.clone());
    let ids = threads.iter().map(|t| t.to_string()).collect::<Vec<_>>().join(", ");
    let input = format!("{system}\n\n[conversations, sent in batches of {}: {ids}]", s.batch_size);
    let target_json = serde_json::json!({ "threads": threads, "folder": role.as_str() }).to_string();
    let row = (
        prompt.id.clone(),
        variant.map(|c| c.to_string()),
        if matches!(prompt.source, PromptSource::Builtin) { "builtin" } else { "user" },
        prompt.hash,
        provider.name(),
        model.clone(),
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
            scope: "selection",
            target: &target_json,
            lang: &row.6,
            input: &input,
        })
    })
    .await?;
    let max_output_tokens = prompt.max_output_tokens;
    let context = Context { prompt, target: Target::Selection, findings: Vec::new() };
    core.ai.running.lock().unwrap().insert(run_id, Running { abort: None, context });
    let batch = Batch { system, model, max_output_tokens, threads, targets, size: s.batch_size as usize };
    let task = tokio::spawn(execute_selection(core.clone(), run_id, batch, provider));
    if let Some(r) = core.ai.running.lock().unwrap().get_mut(&run_id) {
        r.abort = Some(task.abort_handle());
    }
    Ok(AiRunResult { run_id, streamed: false })
}

/// What the batches of a run on several conversations are made of.
struct Batch {
    system: String,
    model: String,
    max_output_tokens: Option<u32>,
    threads: Vec<i64>,
    targets: Vec<FolderRole>,
    size: usize,
}

/// The conversations of a batch as the model sees them: the latest mail
/// received of each (the latest of all when every one is the user's),
/// its authentication and findings, its sender. A conversation gone, or
/// whose mail is not downloaded yet, is left out of the batch.
async fn items(core: &Arc<Core>, threads: Vec<i64>) -> Result<Vec<nm_ai::message::Item>, RpcError> {
    let declared = crate::drafts::declared_addresses(core);
    with_reader(core, move |st| {
        let own = own_addresses(st, declared.clone())?;
        let mut out = Vec::new();
        for t in threads {
            let Ok((_, messages)) = st.get_thread(t, &declared) else { continue };
            let is_mine = |m: &MessageView| m.from_addr.as_deref().is_some_and(|a| own.contains(&a.to_lowercase()));
            let Some(m) = messages.iter().rev().find(|m| !is_mine(m)).or(messages.last()) else { continue };
            if m.body_text.is_none() {
                continue;
            }
            let addr = m.from_addr.clone().unwrap_or_default().to_lowercase();
            out.push(nm_ai::message::Item {
                thread_id: t,
                mail: mail_of(m, is_mine(m)),
                auth: m.auth_result.clone(),
                findings: findings(m),
                sender: sender_history(st, &addr)?.into_iter().skip(1).collect(),
            });
        }
        Ok(out)
    })
    .await
}

/// The task of a run on several conversations (`run_selection`).
async fn execute_selection(core: Arc<Core>, run_id: i64, b: Batch, provider: Provider) {
    let Ok(_slot) = core.ai.slots.clone().acquire_owned().await else { return };
    let total = b.threads.len() as u64;
    let roles: Vec<&str> = b.targets.iter().map(|t| t.as_str()).collect();
    let (mut done, mut failed) = (0u64, 0u64);
    let mut outputs: Vec<String> = Vec::new();
    let mut found: Vec<nm_ai::answer::Suggestion> = Vec::new();
    let (mut input_tokens, mut output_tokens, mut cost, mut billed, mut duration) = (0u64, 0u64, None::<f64>, false, 0u64);
    for chunk in b.threads.chunks(b.size.max(1)) {
        let items = match items(&core, chunk.to_vec()).await {
            Ok(i) => i,
            Err(e) => {
                if core.ai.running.lock().unwrap().remove(&run_id).is_some() {
                    fail(&core, run_id, AiError::Failed(e.message), None).await;
                }
                return;
            }
        };
        if !items.is_empty() {
            let ids: Vec<i64> = items.iter().map(|i| i.thread_id).collect();
            let request = AiRequest {
                system: b.system.clone(),
                message: format!("{}\n\n{}", nm_ai::message::selection_items(&items), nm_ai::message::targets(&roles)),
                model: b.model.clone(),
                schema: Output::Targets.schema(&b.targets),
                max_output_tokens: b.max_output_tokens,
            };
            match provider.run(&request, |_| {}).await {
                // The provider itself failed (not signed in, the usage
                // limit, a timeout): the next batches would fail the same.
                Err(e) => {
                    if core.ai.running.lock().unwrap().remove(&run_id).is_some() {
                        fail(&core, run_id, e, Some(outputs.join("\n"))).await;
                    }
                    return;
                }
                Ok(o) => {
                    input_tokens += o.usage.input_tokens;
                    output_tokens += o.usage.output_tokens;
                    cost = Some(cost.unwrap_or(0.0) + o.cost_usd.unwrap_or(0.0));
                    billed |= o.billed;
                    duration += o.duration_ms.unwrap_or(0);
                    outputs.push(o.text.clone());
                    let sent = Sent { segments: 0, threads: &ids, targets: &b.targets, sender: None };
                    match answer::read(Output::Targets, &o, &sent) {
                        Ok(Answer::Targets(list)) => {
                            let rows: Vec<(i64, String, String)> =
                                list.iter().map(|s| (s.thread, s.target.as_str().to_string(), s.reason.clone())).collect();
                            let stored = with_store(&core, move |st| {
                                let new: Vec<NewSuggestion> = rows
                                    .iter()
                                    .map(|(t, target, reason)| NewSuggestion { thread_id: *t, target, reason })
                                    .collect();
                                st.ai_suggestions_add(run_id, &new)
                            })
                            .await;
                            match stored {
                                Ok(()) => {
                                    let thread_ids = list.iter().map(|s| s.thread).collect();
                                    core.events.lock().unwrap().push(Event::AiSuggestions { run_id, thread_ids });
                                    found.extend(list);
                                }
                                Err(e) => {
                                    eprintln!("ai : suggestions de l'exécution {run_id} non enregistrées : {}", e.message);
                                    failed += 1;
                                }
                            }
                        }
                        // One batch answered badly: the others may not.
                        _ => failed += 1,
                    }
                }
            }
        }
        done += chunk.len() as u64;
        core.events.lock().unwrap().push_transient(Event::AiProgress { run_id, done, total });
    }
    let Some(running) = core.ai.running.lock().unwrap().remove(&run_id) else { return };
    if found.is_empty() && failed > 0 {
        let e = AiError::InvalidOutput("no batch gave targets that could be read".into());
        return fail(&core, run_id, e, Some(outputs.join("\n"))).await;
    }
    let output = outputs.join("\n");
    let stored = with_store(&core, move |st| {
        st.ai_run_done(
            run_id,
            &RunDone {
                output: &output,
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                cost_usd: cost,
                billed,
                duration_ms: Some(duration),
            },
        )
    })
    .await;
    if let Err(e) = stored {
        eprintln!("ai : exécution {run_id} non enregistrée : {}", e.message);
    }
    core.ai.finished.lock().unwrap().insert(run_id, Finished { context: running.context, answer: Answer::Targets(found), failed_batches: failed, saved: false });
    core.events.lock().unwrap().push(Event::AiDone { run_id });
}

/// `ai_suggestions`.
pub(crate) async fn suggestions(core: &Arc<Core>, p: AiSuggestionsParams) -> Result<Vec<AiSuggestionView>, RpcError> {
    let rows = with_reader(core, move |st| st.ai_suggestions_for(&p.thread_ids)).await?;
    Ok(rows
        .into_iter()
        .filter_map(|r| {
            Some(AiSuggestionView { run_id: r.run_id, thread_id: r.thread_id, target: FolderRole::parse(&r.target)?, reason: r.reason })
        })
        .collect())
}

/// `ai_dismiss_suggestion`.
pub(crate) async fn dismiss_suggestion(core: &Arc<Core>, p: AiDismissSuggestionParams) -> Result<CommandResult, RpcError> {
    with_store(core, move |st| st.ai_suggestion_dismiss(p.run_id, p.thread_id)).await?;
    Ok(CommandResult { changed: 0 })
}

/// "Name <address>", or what there is of it.
fn label(name: Option<&str>, addr: Option<&str>) -> String {
    match (name.map(str::trim).filter(|n| !n.is_empty()), addr) {
        (Some(n), Some(a)) => format!("{n} <{a}>"),
        (None, Some(a)) => a.to_string(),
        (Some(n), None) => n.to_string(),
        (None, None) => "(unknown sender)".to_string(),
    }
}

/// The offset of this Mac's time zone at the time `at` (a Unix time), in
/// seconds, for the dates given to the model: "next Tuesday" is read from
/// the mail's local date. At that time, not now: a mail of June is in
/// summer time even when read in November.
fn local_offset(at: i64) -> i32 {
    let now = at as libc::time_t;
    // Rust note: `localtime_r` is a C function that fills a struct we own;
    // `unsafe` because the compiler cannot check it, and `zeroed` gives the
    // struct a valid starting value (every field of `tm` may be zero).
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let ok = unsafe { !libc::localtime_r(&now, &mut tm).is_null() };
    if ok { tm.tm_gmtoff as i32 } else { 0 }
}

/// A mail of the reader as the inputs see it: what the sender wrote,
/// interleaved quotes marked `> `, without the quoted history at its end
/// or the copies of earlier mails (`docs/thread-display.md`).
fn mail_of(m: &MessageView, mine: bool) -> Mail {
    let text = if m.segments.is_empty() {
        m.body_text.clone().unwrap_or_default()
    } else {
        m.segments
            .iter()
            .filter(|s| s.kind != "quote_tail" && s.kind != "copy")
            .map(|s| {
                if s.kind == "quote" {
                    s.text.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
                } else {
                    s.text.clone()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let addresses = |v: &[nm_api::AddressView]| v.iter().map(|a| label(a.name.as_deref(), Some(&a.addr))).collect();
    Mail {
        from: label(m.from_name.as_deref(), m.from_addr.as_deref()),
        to: addresses(&m.to),
        cc: addresses(&m.cc),
        reply_to: addresses(&m.reply_to),
        date: {
            let at = m.date_sent.unwrap_or(m.date_received);
            nm_core::compose::rfc5322_date(at, local_offset(at))
        },
        subject: m.subject.clone(),
        text,
        attachments: m.attachments.iter().map(|a| a.filename.clone().unwrap_or_else(|| a.mime_type.clone())).collect(),
        mine,
    }
}

/// The host of an `http` or `https` URL, lowercase, without user, port,
/// path; `None` for another scheme.
fn url_host(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') { host.split(']').next().map(|h| format!("{h}]"))? } else { host.split(':').next()?.to_string() };
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// A site named by the text of a link ("www.mybank.com", "https://…"),
/// lowercase; `None` for a text that names no site ("Click here").
fn shown_host(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches('/');
    if t.contains(char::is_whitespace) || !t.contains('.') {
        return None;
    }
    if let Some(h) = url_host(t) {
        return Some(h);
    }
    let host = t.split(['/', '?', '#']).next()?.to_lowercase();
    // A domain: letters, digits, dashes and dots, a letter at the end.
    let ok = host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && host.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    ok.then_some(host)
}

/// The Reply-To addresses that send a reply elsewhere than the sender, as
/// findings: the classic phishing where the From is the bank's (and may
/// even pass authentication, sent through a mailing service) while the
/// reply goes to the swindler's mailbox.
///
/// - Another registrable domain than the From's (`mabanque.fr` and
///   `gmail.com`) is one. A subdomain of the same organization
///   (`news.mabanque.fr` replying to `support@mabanque.fr`) is not:
///   domains are compared as `nm_store::auth::organizational_domain`.
/// - On a public mail provider (`domaines-publics.md`), the same domain
///   says nothing of the person: `alice@gmail.com` replying to
///   `bob@gmail.com` is another mailbox, hence another person. Only the
///   very same address is fine there.
///
/// No From, or a Reply-To that is the From itself: nothing.
fn reply_elsewhere(from: Option<&str>, reply_to: &[&str]) -> Vec<String> {
    use nm_store::auth::organizational_domain as org;
    let Some(from) = from.map(|a| a.trim().to_lowercase()) else { return Vec::new() };
    let Some(from_domain) = nm_store::screening::domain_of(&from) else { return Vec::new() };
    let mut out = Vec::new();
    for addr in reply_to {
        let addr = addr.trim().to_lowercase();
        if addr == from {
            continue;
        }
        let Some(domain) = nm_store::screening::domain_of(&addr) else { continue };
        let text = if org(&domain) != org(&from_domain) {
            format!("replies go to {addr}, another domain than the sender's ({from_domain})")
        } else if nm_store::sender_kind::is_public_domain(&org(&domain)) {
            format!("replies go to {addr}, another mailbox than the sender's ({from}) on a public mail provider")
        } else {
            continue;
        };
        if !out.contains(&text) {
            out.push(text);
        }
    }
    out
}

fn is_ip(host: &str) -> bool {
    host.starts_with('[') || (host.split('.').count() == 4 && host.split('.').all(|p| p.parse::<u8>().is_ok()))
}

/// What the core finds by itself in a mail, for the security check
/// (`ia.md`, section 6, "Plancher"): a failed authentication, a Reply-To
/// that sends the reply elsewhere than the sender, a link that shows one
/// site and opens another, a link to a bare IP address.
fn findings(m: &MessageView) -> Vec<Finding> {
    let mut out: Vec<Finding> = Vec::new();
    if m.auth_result == "fail" {
        out.push(Finding { text: "the sender's authentication failed (SPF, DKIM, DMARC)".into(), strong: true });
    }
    let reply_to: Vec<&str> = m.reply_to.iter().map(|a| a.addr.as_str()).collect();
    for text in reply_elsewhere(m.from_addr.as_deref(), &reply_to) {
        out.push(Finding { text, strong: true });
    }
    for link in &m.links {
        let Some(real) = url_host(&link.url) else { continue };
        let text = if is_ip(&real) {
            format!("a link opens the bare IP address {real}")
        } else {
            let Some(shown) = link.text.as_deref().and_then(shown_host) else { continue };
            let org = nm_store::auth::organizational_domain;
            if org(&shown) == org(&real) {
                continue;
            }
            format!("a link shows {shown} but opens {real}")
        };
        if !out.iter().any(|f| f.text == text) {
            out.push(Finding { text, strong: true });
        }
        if out.len() >= 10 {
            break;
        }
    }
    out
}

/// What the mailbox knows of a sender: the decision on the address and on
/// its domain, how many mails it sent, whether the user wrote to it, and
/// whether it is a person or a robot (`expediteurs.md`).
fn sender_history(st: &nm_store::Store, addr: &str) -> Result<Vec<String>, nm_store::StoreError> {
    use nm_store::rusqlite::OptionalExtension;
    let mut lines = vec![format!("Address: {addr}")];
    let row: Option<(Option<String>, i64, i64, Option<String>)> = st
        .conn()
        .query_row(
            "SELECT screening_target, written_to, sender_count, coalesce(sender_kind_user, sender_kind)
               FROM address WHERE addr = ?1",
            [addr],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    match row {
        None => lines.push("Never seen before this mail.".into()),
        Some((decision, written_to, count, kind)) => {
            lines.push(format!("Decision on the address: {}", decision.as_deref().unwrap_or("none")));
            lines.push(format!("Mails received from it: {count}"));
            lines.push(format!("The user has written to it: {}", if written_to != 0 { "yes" } else { "no" }));
            lines.push(format!("Seen as: {}", kind.as_deref().unwrap_or("unknown")));
        }
    }
    if let Some(domain) = nm_store::screening::domain_of(addr) {
        let decision: Option<Option<String>> = st
            .conn()
            .query_row("SELECT screening_target FROM domain WHERE domain = ?1", [&domain], |r| r.get(0))
            .optional()?;
        lines.push(format!("Decision on its domain {domain}: {}", decision.flatten().as_deref().unwrap_or("none")));
    }
    Ok(lines)
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
async fn execute(core: Arc<Core>, run_id: i64, mut request: AiRequest, provider: Provider, deferred: Option<Deferred>) {
    // Rust note: `acquire_owned` waits for a place and gives a permit that
    // frees it when dropped, at the end of this function, whatever happens.
    let Ok(_slot) = core.ai.slots.clone().acquire_owned().await else { return };
    if let Some(d) = deferred {
        match crate::extract::text(&d.path, d.kind).await {
            Err(e) => {
                // Nothing was sent: the input was not good, as for an empty
                // draft, but known only now.
                if core.ai.running.lock().unwrap().remove(&run_id).is_some() {
                    fail(&core, run_id, AiError::BadInput(format!("{}: {e}", d.name)), None).await;
                }
                return;
            }
            Ok(text) => {
                let attachment = nm_ai::message::attachment_text(&d.name, d.kind.as_str(), &text);
                request.message =
                    d.sections.into_iter().map(|s| s.unwrap_or_else(|| attachment.clone())).collect::<Vec<_>>().join("\n\n");
                let input = format!("{}\n\n{}", request.system, request.message);
                if let Err(e) = with_store(&core, move |st| st.ai_run_set_input(run_id, &input)).await {
                    eprintln!("ai : message de l'exécution {run_id} non enregistré : {}", e.message);
                }
            }
        }
    }
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
    let sent = Sent { segments: context.target.segments(), threads: &[], targets: &[], sender: context.target.sender() };
    match answer::read(context.prompt.output, &outcome, &sent) {
        Err(e) => fail(&core, run_id, e, Some(outcome.text.clone())).await,
        Ok(answer) => {
            // The model may have missed what the core found itself.
            let answer = match answer {
                Answer::Verdict(v) => Answer::Verdict(answer::apply_floor(v, &context.findings)),
                other => other,
            };
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
            core.ai.finished.lock().unwrap().insert(run_id, Finished { context, answer, failed_batches: 0, saved: false });
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

/// The deep link of the mail a run read (`docs/deep-links.md`), put in its
/// calendar files and contact cards: from the calendar or the address
/// book, one click goes back to the mail.
fn mail_link(t: &Target) -> Option<String> {
    match t {
        Target::Mail { message, .. } | Target::Attachment { message } => Some(format!("nicemailer://m/{message}")),
        _ => None,
    }
}

/// The proposal of a finished run, as the client shows it.
fn proposal(run_id: i64, f: &Finished) -> Option<AiProposal> {
    match (&f.answer, &f.context.target) {
        (Answer::Segments(new), Target::Draft { body: old_body, ranges, .. }) => {
            let body = nm_core::segments::splice(old_body, ranges, new).ok()?;
            let segments: Vec<AiSegmentChange> = ranges
                .iter()
                .zip(new)
                .map(|(r, n)| {
                    let old = &old_body[r.clone()];
                    AiSegmentChange {
                        start: utf16_of_byte(old_body, r.start),
                        end: utf16_of_byte(old_body, r.end),
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
            let unchanged = body == *old_body;
            Some(AiProposal::Replace { segments, body, unchanged })
        }
        (Answer::Subjects(items), _) => Some(AiProposal::Subjects { items: items.clone() }),
        (Answer::Text(text), _) => Some(AiProposal::Text { text: text.clone() }),
        (Answer::Summary { summary, actions }, _) => Some(AiProposal::Summary {
            summary: summary.clone(),
            actions: actions
                .iter()
                .map(|a| AiActionView { task: a.task.clone(), owner: a.owner.clone(), due: a.due.clone() })
                .collect(),
        }),
        (Answer::Verdict(v), _) => {
            Some(AiProposal::Verdict { risk: v.risk.as_str().into(), reasons: v.reasons.clone(), raised: v.raised })
        }
        (Answer::Targets(list), _) => Some(AiProposal::Targets {
            items: list
                .iter()
                .map(|s| AiSuggestionView { run_id, thread_id: s.thread, target: s.target, reason: s.reason.clone() })
                .collect(),
            failed_batches: f.failed_batches,
        }),
        (Answer::Events(list), _) => Some(AiProposal::Events {
            items: list
                .iter()
                .map(|e| AiEventView {
                    title: e.title.clone(),
                    start: e.start.iso(),
                    end: e.end.map(|w| w.iso()).unwrap_or_default(),
                    location: e.location.clone(),
                    details: e.details.clone(),
                })
                .collect(),
        }),
        (Answer::Contact(c), _) => Some(AiProposal::Contact {
            card: AiContactView {
                name: c.name.clone(),
                given: c.given.clone(),
                family: c.family.clone(),
                organization: c.organization.clone(),
                title: c.title.clone(),
                emails: c.emails.clone(),
                phones: c
                    .phones
                    .iter()
                    .map(|p| AiPhoneView { kind: p.kind.as_str().to_string(), number: p.number.clone() })
                    .collect(),
                address: AiAddressView {
                    street: c.address.street.clone(),
                    locality: c.address.locality.clone(),
                    postcode: c.address.postcode.clone(),
                    region: c.address.region.clone(),
                    country: c.address.country.clone(),
                },
                urls: c.urls.clone(),
                dropped: c.dropped.clone(),
            },
        }),
        (Answer::Choices(items), _) => Some(AiProposal::Choices {
            items: items.clone(),
            then: match f.context.prompt.then {
                Some(nm_ai::prompt::Then::Insert) => "insert".into(),
                Some(nm_ai::prompt::Then::Replace) => "replace".into(),
                _ => "reply".into(),
            },
        }),
        _ => None,
    }
}

/// Whether a proposal is only read (a translation, a summary, a verdict):
/// nothing to apply, and closing it is no decision on it.
fn read_only(a: &Answer) -> bool {
    // Targets are decided one by one, by the moves that follow.
    // A mail without a date: nothing to save.
    matches!(a, Answer::Text(_) | Answer::Summary { .. } | Answer::Verdict(_) | Answer::Targets(_))
        || matches!(a, Answer::Events(list) if list.is_empty())
}

/// `ai_get_run`.
pub(crate) async fn get_run(core: &Arc<Core>, p: AiRunIdParams) -> Result<AiRunView, RpcError> {
    let id = p.run_id;
    let row = with_reader(core, move |st| st.ai_run(id))
        .await?
        .ok_or_else(|| RpcError::new(error_code::NOT_FOUND, format!("no AI run {id}")))?;
    let proposal = core.ai.finished.lock().unwrap().get(&id).and_then(|f| proposal(id, f));
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
    // Any refusal below puts the proposal back: a client told "the draft
    // changed" can still dismiss it, and a retry with the right revision or
    // choice still finds it.
    let put_back = |f: Finished, e: RpcError| {
        core.ai.finished.lock().unwrap().insert(p.run_id, f);
        Err(e)
    };
    let run_id = p.run_id;
    // A choice the client applies itself (a quick reply starts a reply):
    // the core records it and gives the text back.
    // A choice that replaces the passage selected (other wordings) is not
    // one: the core puts it in the draft below, as a
    // proofreading, once sure the draft is still the text the AI read.
    // An appointment: its calendar file, for the client to save. The
    // proposal stays: a mail with three dates may go into the calendar
    // three times. The first file saved accepts the run; closing the
    // panel afterwards (`ai_dismiss`) does not turn it down.
    if let Answer::Events(list) = &finished.answer {
        let Some((n, event)) = p.choice.and_then(|i| list.get(i).map(|e| (i, e))) else {
            return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "choose one of the appointments (choice)"));
        };
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let calendar = AiFile {
            filename: nm_ai::ics::file_name(event),
            // The same run and appointment give the same uid: saved twice,
            // the calendar updates the event instead of adding it again.
            content: nm_ai::ics::calendar(
                event,
                &format!("nicemailer-ai-{run_id}-{}@nicemailer", n + 1),
                stamp,
                mail_link(&finished.context.target).as_deref(),
            ),
        };
        if !finished.saved {
            if let Err(e) = with_store(core, move |st| st.ai_run_decide(run_id, Decision::Accepted)).await {
                return put_back(finished, e);
            }
        }
        core.ai.finished.lock().unwrap().insert(run_id, Finished { saved: true, ..finished });
        return Ok(AiApplyResult { draft: None, text: None, calendar: Some(calendar), contact: None });
    }
    // The sender's card: its vCard file, for the client to save. Unlike
    // the appointments, there is one card: the run is accepted and ends
    // here, the proposal is not put back.
    if let Answer::Contact(contact) = &finished.answer {
        let stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let file = AiFile {
            filename: nm_ai::vcard::file_name(contact),
            content: nm_ai::vcard::card(
                contact,
                &format!("nicemailer-ai-{run_id}@nicemailer"),
                stamp,
                mail_link(&finished.context.target).as_deref(),
            ),
        };
        if let Err(e) = with_store(core, move |st| st.ai_run_decide(run_id, Decision::Accepted)).await {
            return put_back(finished, e);
        }
        return Ok(AiApplyResult { draft: None, text: None, calendar: None, contact: Some(file) });
    }
    let replaces = finished.context.prompt.then == Some(nm_ai::prompt::Then::Replace);
    if let (Answer::Choices(items), false) = (&finished.answer, replaces) {
        let Some(text) = p.choice.and_then(|i| items.get(i)).cloned() else {
            return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "choose one of the texts (choice)"));
        };
        if let Err(e) = with_store(core, move |st| st.ai_run_decide(run_id, Decision::Accepted)).await {
            return put_back(finished, e);
        }
        return Ok(AiApplyResult { draft: None, text: Some(text), calendar: None, contact: None });
    }
    let Target::Draft { draft_id, revision, body, ranges } = finished.context.target.clone() else {
        return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "nothing to apply: close it with ai_dismiss"));
    };
    // The text the AI saw is the text of that revision: a later one may
    // hold words typed since, which applying would erase.
    if p.revision != Some(revision) {
        let mut err = RpcError::new(error_code::REVISION_CONFLICT, "the draft changed since the AI read it: run it again");
        err.data = Some(serde_json::json!({ "revision": revision }));
        return put_back(finished, err);
    }
    let edit = match &finished.answer {
        Answer::Segments(new) => match nm_core::segments::splice(&body, &ranges, new) {
            Ok(body) => DraftEdit { body: Some(body), ..Default::default() },
            Err(m) => return put_back(finished, RpcError::new(error_code::INTERNAL_ERROR, m)),
        },
        Answer::Choices(items) => match p.choice.and_then(|i| items.get(i)) {
            Some(text) => match nm_core::segments::splice(&body, &ranges, std::slice::from_ref(text)) {
                Ok(body) => DraftEdit { body: Some(body), ..Default::default() },
                Err(m) => return put_back(finished, RpcError::new(error_code::INTERNAL_ERROR, m)),
            },
            None => return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "choose one of the texts (choice)")),
        },
        Answer::Subjects(items) => match p.choice.and_then(|i| items.get(i)) {
            Some(subject) => DraftEdit { subject: Some(subject.clone()), ..Default::default() },
            None => return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "choose one of the subjects (choice)")),
        },
        _ => return put_back(finished, RpcError::new(error_code::INVALID_PARAMS, "this proposal cannot be applied")),
    };
    let saved = with_store(core, move |st| {
        st.edit_draft(draft_id, revision, &edit, source)?;
        st.ai_run_decide(run_id, Decision::Accepted)?;
        st.get_draft(draft_id)
    })
    .await;
    match saved {
        Ok(draft) => Ok(AiApplyResult { draft: Some(draft), text: None, calendar: None, contact: None }),
        Err(e) => put_back(finished, e),
    }
}

/// `ai_dismiss`: the proposal is turned down, or a proposal only read is
/// closed (no decision then); a run still going is cancelled.
pub(crate) async fn dismiss(core: &Arc<Core>, p: AiRunIdParams) -> Result<CommandResult, RpcError> {
    if core.ai.running.lock().unwrap().contains_key(&p.run_id) {
        return cancel(core, p).await;
    }
    let Some(finished) = core.ai.finished.lock().unwrap().remove(&p.run_id) else {
        return Err(RpcError::new(error_code::NOT_FOUND, format!("AI run {} has no proposal", p.run_id)));
    };
    if !read_only(&finished.answer) && !finished.saved {
        let id = p.run_id;
        with_store(core, move |st| st.ai_run_decide(id, Decision::Rejected)).await?;
    }
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
    fn hosts_are_read_from_urls_and_link_texts() {
        assert_eq!(url_host("https://user@www.Bank.fr:8443/a?b#c").as_deref(), Some("www.bank.fr"));
        assert_eq!(url_host("http://[2001:db8::1]/x").as_deref(), Some("[2001:db8::1]"));
        assert_eq!(url_host("mailto:a@b.fr"), None);
        assert_eq!(shown_host("www.mabanque.fr").as_deref(), Some("www.mabanque.fr"));
        assert_eq!(shown_host("https://mabanque.fr/login").as_deref(), Some("mabanque.fr"));
        assert_eq!(shown_host("mabanque.fr/espace").as_deref(), Some("mabanque.fr"));
        for not_a_site in ["Cliquez ici", "Voir ma facture", "v2.0", "1.5", "a.b c"] {
            assert_eq!(shown_host(not_a_site), None, "{not_a_site}");
        }
        assert!(is_ip("203.0.113.9") && is_ip("[::1]") && !is_ip("1.2.3") && !is_ip("www.a.fr"));
    }

    fn message(auth: &str, links: &[(&str, Option<&str>)]) -> MessageView {
        message_replying_to(auth, links, &[])
    }

    fn message_replying_to(auth: &str, links: &[(&str, Option<&str>)], reply_to: &[&str]) -> MessageView {
        let reply_to: Vec<serde_json::Value> = reply_to.iter().map(|a| serde_json::json!({"addr": a, "name": null})).collect();
        let links: Vec<serde_json::Value> =
            links.iter().map(|(u, t)| serde_json::json!({"url": u, "text": t})).collect();
        serde_json::from_value(serde_json::json!({
            "id": 1, "folder_id": 1, "folder_role": null, "subject": "s", "from_name": null, "from_addr": "a@b.fr",
            "to": [], "cc": [], "reply_to": reply_to, "date_sent": null, "date_received": 0, "body_text": "x", "is_seen": true,
            "is_flagged": false, "is_answered": false, "has_attachments": false, "auth_result": auth,
            "body_html": null, "has_remote_content": false, "from_body_format": null, "segments": [],
            "only_quotes": false, "expanded": true, "show_recipients": false, "attachments": [],
            "unsubscribe": null, "tracking_pixels": 0, "links": links, "verification_code": null,
        }))
        .unwrap()
    }

    #[test]
    fn findings_name_what_lies() {
        let text = |m: &MessageView| findings(m).into_iter().map(|f| f.text).collect::<Vec<_>>();
        // A link to the same organization under another name is fine.
        assert!(text(&message("pass", &[("https://espace.mabanque.fr/x", Some("www.mabanque.fr"))])).is_empty());
        assert!(text(&message("pass", &[("https://other.example/x", Some("Cliquez ici"))])).is_empty());
        let t = text(&message("fail", &[
            ("http://evil.example/login", Some("www.mabanque.fr")),
            ("http://evil.example/login", Some("www.mabanque.fr")),
            ("http://203.0.113.9/steal", Some("Cliquez")),
        ]));
        assert_eq!(t, [
            "the sender's authentication failed (SPF, DKIM, DMARC)",
            "a link shows www.mabanque.fr but opens evil.example",
            "a link opens the bare IP address 203.0.113.9",
        ]);
        assert!(findings(&message("fail", &[])).iter().all(|f| f.strong));
        // A Reply-To elsewhere, strong, after the authentication and
        // before the links.
        let m = message_replying_to("fail", &[("http://203.0.113.9/x", Some("Cliquez"))], &["x@evil.example"]);
        assert_eq!(text(&m), [
            "the sender's authentication failed (SPF, DKIM, DMARC)",
            "replies go to x@evil.example, another domain than the sender's (b.fr)",
            "a link opens the bare IP address 203.0.113.9",
        ]);
        assert!(findings(&m).iter().all(|f| f.strong));
    }

    #[test]
    fn a_reply_to_elsewhere_is_found() {
        let f = |from: Option<&str>, rt: &[&str]| reply_elsewhere(from, rt);
        assert_eq!(
            f(Some("support@mabanque.fr"), &["Support-Mabanque@Gmail.com"]),
            ["replies go to support-mabanque@gmail.com, another domain than the sender's (mabanque.fr)"]
        );
        // The same organization: a subdomain, another address of it.
        assert!(f(Some("news@info.mabanque.fr"), &["support@mabanque.fr"]).is_empty());
        assert!(f(Some("a@mabanque.co.uk"), &["b@x.mabanque.co.uk"]).is_empty());
        // The From itself, whatever its case; no From; not an address.
        assert!(f(Some("Alice@Gmail.com"), &["alice@gmail.com"]).is_empty());
        assert!(f(None, &["a@evil.example"]).is_empty());
        assert!(f(Some("a@mabanque.fr"), &["not an address"]).is_empty());
        // A public provider: another mailbox there is another person.
        assert_eq!(
            f(Some("alice@gmail.com"), &["bob@gmail.com"]),
            ["replies go to bob@gmail.com, another mailbox than the sender's (alice@gmail.com) on a public mail provider"]
        );
        // Twice the same: one finding.
        assert_eq!(f(Some("a@mabanque.fr"), &["x@evil.example", "X@evil.example"]).len(), 1);
    }

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
