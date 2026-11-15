//! The history of AI runs (`specifications/ia.md`, section 9; tables
//! `ai_run` and `ai_suggestion` of `schema.sql`, version 13).
//!
//! A run gets its row when it starts (`running`), and the row is completed
//! when it ends (`done`, `failed`, `cancelled`), then when the user decides
//! (`accepted`, `rejected`, `partial`). The store only keeps what it is
//! told: it knows nothing of prompts or providers (`nm-ai` does), so this
//! crate does not depend on that one.
//!
//! Each change checks the state it starts from, in its `WHERE`: a run is
//! finished once, decided once, and only when done. A second answer for the
//! same run (two clients, a retry) is refused instead of overwriting the
//! first.

use nm_core::Source;
use rusqlite::{OptionalExtension, params};

use crate::{Result, Store, StoreError};

/// A run about to start.
#[derive(Debug, Clone)]
pub struct NewRun<'a> {
    pub created_by: Source,
    pub prompt_id: &'a str,
    pub variant: Option<&'a str>,
    /// `builtin` or `user`.
    pub prompt_source: &'a str,
    pub prompt_hash: &'a [u8; 32],
    pub provider: &'a str,
    pub model: &'a str,
    pub scope: &'a str,
    /// JSON: `{"draft": 12}`, `{"threads": [3, 4]}`…
    pub target: &'a str,
    pub lang: &'a str,
    /// The system prompt and the message sent, as they were.
    pub input: &'a str,
}

/// The end of a successful run.
#[derive(Debug, Clone, Default)]
pub struct RunDone<'a> {
    /// The raw answer.
    pub output: &'a str,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub billed: bool,
    pub duration_ms: Option<u64>,
}

/// How a run ended without an answer to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEnd {
    Failed,
    Cancelled,
}

/// What the user did with a proposal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accepted,
    Rejected,
    /// Some of the suggestions of a batch were followed.
    Partial,
}

impl Decision {
    fn as_str(self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Rejected => "rejected",
            Decision::Partial => "partial",
        }
    }
}

/// One row of `ai_run`, as read back.
#[derive(Debug, Clone, PartialEq)]
pub struct AiRun {
    pub id: i64,
    pub date_created: i64,
    pub created_by: String,
    pub prompt_id: String,
    pub variant: Option<String>,
    pub prompt_source: String,
    pub prompt_hash: Vec<u8>,
    pub provider: String,
    pub model: String,
    pub scope: String,
    pub target: String,
    pub lang: String,
    pub input: Option<String>,
    pub output: Option<String>,
    pub state: String,
    pub error: Option<String>,
    pub decision: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub billed: bool,
    pub duration_ms: Option<i64>,
    pub date_purged: Option<i64>,
}

/// A suggested target of a batch, for one conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSuggestion<'a> {
    pub thread_id: i64,
    /// A folder role (`inbox`, `newsletter`…).
    pub target: &'a str,
    pub reason: &'a str,
}

/// A suggestion waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub run_id: i64,
    pub thread_id: i64,
    pub target: String,
    pub reason: String,
}

/// The text of a run left `running` by a stop of the core.
pub const ABANDONED: &str = "ai_failed: the core stopped during the run";

impl Store {
    /// Records a run as it starts; returns its id.
    pub fn ai_run_start(&mut self, r: &NewRun) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO ai_run (created_by, prompt_id, variant, prompt_source, prompt_hash, provider, model,
                                 scope, target, lang, input, state, billed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'running', 0)",
            params![
                r.created_by.as_str(),
                r.prompt_id,
                r.variant,
                r.prompt_source,
                &r.prompt_hash[..],
                r.provider,
                r.model,
                r.scope,
                r.target,
                r.lang,
                r.input
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The message sent, once known: the text of an attachment is read in
    /// the run's task, after the row is written, so the row first holds
    /// the system prompt alone (`nm-daemon`, `ai.rs`, "Attachments").
    pub fn ai_run_set_input(&mut self, id: i64, input: &str) -> Result<()> {
        let n = self.conn.execute("UPDATE ai_run SET input = ?2 WHERE id = ?1 AND state = 'running'", params![id, input])?;
        self.one(n, id, "is not running")
    }

    /// Completes a running run with its answer.
    pub fn ai_run_done(&mut self, id: i64, d: &RunDone) -> Result<()> {
        // Rust note: `u64` does not fit SQLite's signed integers by itself;
        // `i64::try_from` refuses a value too large instead of wrapping it.
        let int = |v: Option<u64>| v.and_then(|n| i64::try_from(n).ok());
        let n = self.conn.execute(
            "UPDATE ai_run SET state = 'done', output = ?2, input_tokens = ?3, output_tokens = ?4,
                    cost_usd = ?5, billed = ?6, duration_ms = ?7
              WHERE id = ?1 AND state = 'running'",
            params![
                id,
                d.output,
                int(d.input_tokens),
                int(d.output_tokens),
                d.cost_usd,
                d.billed as i64,
                int(d.duration_ms)
            ],
        )?;
        self.one(n, id, "is not running")
    }

    /// Ends a running run without an answer to apply. `output` is the raw
    /// answer when there was one (an answer refused as invalid is kept, to
    /// understand why: `ia.md`, section 6).
    pub fn ai_run_end(&mut self, id: i64, end: RunEnd, error: &str, output: Option<&str>) -> Result<()> {
        let state = match end {
            RunEnd::Failed => "failed",
            RunEnd::Cancelled => "cancelled",
        };
        let n = self.conn.execute(
            "UPDATE ai_run SET state = ?2, error = ?3, output = ?4 WHERE id = ?1 AND state = 'running'",
            params![id, state, error, output],
        )?;
        self.one(n, id, "is not running")
    }

    /// Records the user's decision on a finished run, once.
    pub fn ai_run_decide(&mut self, id: i64, decision: Decision) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE ai_run SET decision = ?2, date_decided = unixepoch()
              WHERE id = ?1 AND state = 'done' AND decision IS NULL",
            params![id, decision.as_str()],
        )?;
        self.one(n, id, "has no answer waiting for a decision")
    }

    /// Fails the runs a stop of the core left `running`; called at start,
    /// before any new run. Returns how many.
    pub fn ai_runs_abandoned(&mut self) -> Result<u64> {
        let n = self.conn.execute(
            "UPDATE ai_run SET state = 'failed', error = ?1 WHERE state = 'running'",
            [ABANDONED],
        )?;
        Ok(n as u64)
    }

    /// Forgets the texts sent and received of the runs created before
    /// `before` (a Unix time), keeping the rest of their rows (`ia.md`,
    /// section 9, "Purge"). A run still running is left alone. Returns how
    /// many rows were purged.
    pub fn ai_runs_purge(&mut self, before: i64) -> Result<u64> {
        let n = self.conn.execute(
            "UPDATE ai_run SET input = NULL, output = NULL, date_purged = unixepoch()
              WHERE date_purged IS NULL AND date_created < ?1 AND state <> 'running'",
            [before],
        )?;
        Ok(n as u64)
    }

    /// One run, or `None`.
    pub fn ai_run(&self, id: i64) -> Result<Option<AiRun>> {
        Ok(self
            .conn
            .query_row(
                "SELECT id, date_created, created_by, prompt_id, variant, prompt_source, prompt_hash, provider,
                        model, scope, target, lang, input, output, state, error, decision, input_tokens,
                        output_tokens, cost_usd, billed, duration_ms, date_purged
                   FROM ai_run WHERE id = ?1",
                [id],
                |r| {
                    Ok(AiRun {
                        id: r.get(0)?,
                        date_created: r.get(1)?,
                        created_by: r.get(2)?,
                        prompt_id: r.get(3)?,
                        variant: r.get(4)?,
                        prompt_source: r.get(5)?,
                        prompt_hash: r.get(6)?,
                        provider: r.get(7)?,
                        model: r.get(8)?,
                        scope: r.get(9)?,
                        target: r.get(10)?,
                        lang: r.get(11)?,
                        input: r.get(12)?,
                        output: r.get(13)?,
                        state: r.get(14)?,
                        error: r.get(15)?,
                        decision: r.get(16)?,
                        input_tokens: r.get(17)?,
                        output_tokens: r.get(18)?,
                        cost_usd: r.get(19)?,
                        billed: r.get::<_, i64>(20)? != 0,
                        duration_ms: r.get(21)?,
                        date_purged: r.get(22)?,
                    })
                },
            )
            .optional()?)
    }

    /// Keeps the suggestions of a finished batch, in one transaction.
    pub fn ai_suggestions_add(&mut self, run_id: i64, items: &[NewSuggestion]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut ins = tx.prepare(
                "INSERT INTO ai_suggestion (run_id, thread_id, target, reason) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for s in items {
                ins.execute(params![run_id, s.thread_id, s.target, s.reason])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The suggestions still waiting for the user, for these conversations:
    /// the latest one of each (a conversation sorted twice keeps the newer
    /// suggestion), by thread id.
    pub fn ai_suggestions_for(&self, threads: &[i64]) -> Result<Vec<Suggestion>> {
        let mut out = Vec::new();
        // One statement per thread on the primary key's index: the screen
        // asks for its visible rows, a few dozen.
        let mut q = self.conn.prepare_cached(
            "SELECT run_id, thread_id, target, reason FROM ai_suggestion
              WHERE thread_id = ?1 AND decision IS NULL ORDER BY run_id DESC LIMIT 1",
        )?;
        for t in threads {
            if let Some(s) = q
                .query_row([t], |r| Ok(Suggestion { run_id: r.get(0)?, thread_id: r.get(1)?, target: r.get(2)?, reason: r.get(3)? }))
                .optional()?
            {
                out.push(s);
            }
        }
        Ok(out)
    }

    /// Conversations moved to `target`: their suggestions still waiting
    /// become `accepted` (the suggested target) or `other` (`ia.md`,
    /// section 9). Returns how many.
    pub fn ai_suggestions_resolve(&mut self, threads: &[i64], target: &str) -> Result<usize> {
        let mut n = 0;
        let mut q = self.conn.prepare_cached(
            "UPDATE ai_suggestion SET decision = CASE WHEN target = ?2 THEN 'accepted' ELSE 'other' END
              WHERE thread_id = ?1 AND decision IS NULL",
        )?;
        for t in threads {
            n += q.execute(params![t, target])?;
        }
        Ok(n)
    }

    /// The user set a suggestion aside.
    pub fn ai_suggestion_dismiss(&mut self, run_id: i64, thread_id: i64) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE ai_suggestion SET decision = 'dismissed' WHERE run_id = ?1 AND thread_id = ?2 AND decision IS NULL",
            params![run_id, thread_id],
        )?;
        if n == 1 { Ok(()) } else { Err(StoreError::NotFound(format!("AI suggestion {run_id}/{thread_id}"))) }
    }

    /// Exactly one row was changed, or the run was not in the state needed.
    fn one(&self, changed: usize, id: i64, why: &str) -> Result<()> {
        if changed == 1 {
            return Ok(());
        }
        let exists: bool = self.conn.query_row("SELECT EXISTS (SELECT 1 FROM ai_run WHERE id = ?1)", [id], |r| r.get(0))?;
        Err(if exists {
            StoreError::InvalidParams(format!("AI run {id} {why}"))
        } else {
            StoreError::NotFound(format!("AI run {id}"))
        })
    }
}
