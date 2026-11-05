//! Downloading the bodies (`plan-imap.md`, section 2, "Corps"; phase 3,
//! `docs/ingestion.md`).
//!
//! Every message first enters the database with its headers alone
//! (`pass.rs`). Its body comes here, in this order:
//!
//! 1. the bodies the user is waiting for (a message opened,
//!    [`super::Syncer::prioritize`]);
//! 2. the Inbox, the most recent first;
//! 3. the other folders, the most recent first.
//!
//! Each body is streamed from the server into the raw store
//! (`RawWriter`), parsed from its plain copy, then completed in the
//! database (`Inserter::complete`) in batches: up to 20 bodies, 16 MB or
//! 2 seconds, after one `RawStore::barrier` per batch, so that a row never
//! references a file not yet on stable storage.
//!
//! A body whose size differs from what the server announced is refused
//! (`stockage-brut.md`, section 5, step 2) and not tried again until the
//! core restarts: some servers count `RFC822.SIZE` differently from the
//! bytes they send, and retrying in a loop would only download it again
//! and again. The count is in `sync_status` (`bodies_refused`).

use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use nm_api::Event;
use nm_core::Source;
use nm_proto::imap::{MailboxName, Session};
use nm_store::ingest::{ParsedMessage, parse};
use nm_store::insert::{Body, Inserter};
use nm_store::raw::Written;
use nm_store::sync::MissingBody;
use nm_store::{ChangeSet, StoreError};

use super::pass::PassError;
use super::{Control, SyncHost, read, write};

/// A batch of bodies is written when it has this many…
const BATCH_BODIES: usize = 20;
/// …or this many bytes…
const BATCH_BYTES: u64 = 16 << 20;
/// …or is this old.
const BATCH_AGE: Duration = Duration::from_secs(2);

/// A body downloaded and parsed, waiting for its batch to be written.
struct Done {
    id: i64,
    written: Written,
    parsed: ParsedMessage,
}

/// The body queue of an account, kept across connections.
pub(crate) struct Bodies {
    queue: VecDeque<MissingBody>,
    /// The queue must be read again from the database (start, or a pass
    /// brought new messages, which may go before the queue's rest).
    stale: bool,
    pending: Vec<Done>,
    pending_bytes: u64,
    oldest: Option<Instant>,
    /// Folder selected on the session, by id.
    selected: Option<i64>,
    /// Bodies completed since the queue was read, for the progress.
    done: u64,
}

impl Bodies {
    pub(crate) fn new() -> Bodies {
        Bodies { queue: VecDeque::new(), stale: true, pending: Vec::new(), pending_bytes: 0, oldest: None, selected: None, done: 0 }
    }

    /// A new session: no folder selected on it, and the queue read again
    /// from the database. The body being downloaded when the previous
    /// session failed had left the queue: it is found again there.
    pub(crate) fn new_session(&mut self) {
        self.selected = None;
        self.stale = true;
    }

    /// Downloads bodies until none is left (`Ok(false)`), `deadline`
    /// passes (`Ok(true)`: some are left), or the worker must stop.
    pub(crate) async fn run(
        &mut self,
        s: &mut Session,
        host: &dyn SyncHost,
        account_id: i64,
        control: &Control,
        refused: &mut u64,
        deadline: Option<Instant>,
    ) -> Result<bool, PassError> {
        let mut last_progress = Instant::now();
        loop {
            if control.stopped() {
                self.flush(host)?;
                return Err(PassError::Stopped);
            }
            if deadline.is_some_and(|d| Instant::now() >= d) {
                self.flush(host)?;
                return Ok(true);
            }
            if control.reload.swap(false, Ordering::SeqCst) {
                self.stale = true;
            }
            if self.batch_due() {
                self.flush(host)?;
            }
            let Some(next) = self.next(host, account_id, control)? else {
                self.flush(host)?;
                return Ok(false);
            };
            if control.refused.lock().unwrap().contains(&next.id) || self.pending.iter().any(|d| d.id == next.id) {
                continue;
            }
            if self.selected != Some(next.folder_id) {
                // Read-only: fetching a body never changes the folder.
                match s.examine(&MailboxName::new(&next.path), None).await {
                    Ok(_) => self.selected = Some(next.folder_id),
                    Err(e) if e.is_fatal() => return Err(e.into()),
                    Err(_) => {
                        // The folder cannot be opened now (gone?): its
                        // bodies wait for the next reading of the queue.
                        self.queue.retain(|b| b.folder_id != next.folder_id);
                        continue;
                    }
                }
            }
            // An unknown size: fetched in pieces, whatever its size.
            let announced = next.size.and_then(|n| u32::try_from(n).ok());
            let mut w = host.raw().writer(true).map_err(io)?;
            let got = s.fetch_body(next.uid, announced.unwrap_or(u32::MAX), &mut |piece: &[u8]| w.write(piece)).await?;
            match got {
                Err(e) => return Err(io(e)),
                // Gone from the server meanwhile: the next pass deletes it.
                Ok(None) => continue,
                Ok(Some(_)) => {}
            }
            let finished = match w.finish(announced.map(u64::from)) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => {
                    eprintln!("sync: body of message {} refused: {e}", next.id);
                    control.refused.lock().unwrap().insert(next.id);
                    *refused += 1;
                    continue;
                }
                Err(e) => return Err(io(e)),
            };
            let parsed = match &finished.plain {
                Some(plain) => parse(plain),
                None => ParsedMessage::default(),
            };
            self.pending_bytes += finished.written.size;
            self.oldest.get_or_insert_with(Instant::now);
            self.pending.push(Done { id: next.id, written: finished.written, parsed });
            self.done += 1;
            if last_progress.elapsed() >= Duration::from_millis(500) {
                last_progress = Instant::now();
                let total = self.done + self.queue.len() as u64;
                host.transient(Event::SyncProgress { account_id, folder_id: None, stage: "bodies".into(), done: self.done, total });
            }
        }
    }

    fn batch_due(&self) -> bool {
        self.pending.len() >= BATCH_BODIES
            || self.pending_bytes >= BATCH_BYTES
            || self.oldest.is_some_and(|t| t.elapsed() >= BATCH_AGE)
    }

    /// The next body to download: one the user waits for, else the queue,
    /// read again from the database when stale or empty.
    fn next(&mut self, host: &dyn SyncHost, account_id: i64, control: &Control) -> Result<Option<MissingBody>, PassError> {
        loop {
            let wanted = control.priority.lock().unwrap().pop_front();
            let Some(id) = wanted else { break };
            if let Some((acc, b)) = read(host, |st| st.missing_body(id))?
                && acc == account_id
            {
                return Ok(Some(b));
            }
        }
        // Read again when marked stale (start, a pass brought new mail);
        // an empty queue then means no body is missing, until the next
        // pass marks it stale again.
        if self.stale {
            self.queue = read(host, |st| st.bodies_missing(account_id))?.into();
            self.stale = false;
            self.done = 0;
        }
        Ok(self.queue.pop_front())
    }

    /// Writes the bodies downloaded: their files flushed to stable
    /// storage first, then their rows in one transaction.
    fn flush(&mut self, host: &dyn SyncHost) -> Result<(), PassError> {
        if self.pending.is_empty() {
            return Ok(());
        }
        host.raw().barrier().map_err(io)?;
        let pending = std::mem::take(&mut self.pending);
        self.pending_bytes = 0;
        self.oldest = None;
        write(host, |st| {
            let mut ins = Inserter::new(Source::Imap, None);
            let mut changes = ChangeSet::default();
            let tx = st.transaction()?;
            for d in &pending {
                if let Err(e) = ins.complete(&tx, d.id, Body { written: &d.written, parsed: &d.parsed }, &mut changes) {
                    ins.abandon();
                    return Err(e);
                }
            }
            ins.commit(tx, &mut changes)?;
            Ok(((), changes))
        })?;
        Ok(())
    }
}

fn io(e: std::io::Error) -> PassError {
    PassError::Store(StoreError::Io(format!("raw store: {e}")))
}
