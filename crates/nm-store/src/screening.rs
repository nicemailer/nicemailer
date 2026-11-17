//! Screening decisions on senders and the filing of their other mails
//! (`dossiers.md`, "Déplacement et décision sur l'expéditeur" and
//! "Décision sur un expéditeur : rangement des autres mails").
//!
//! A decision is recorded on the `address` (or `domain`) row: it says where
//! the sender's next mails go. Taking or changing one also files the mails
//! already received from that sender, in every account: that is how an
//! imported history gets sorted, decision after decision.
//!
//! Everything here runs inside the caller's transaction, so a move, the
//! decision it implies and the filing it triggers are committed together or
//! not at all.
//!
//! Undo (`dossiers.md`, "Annulation"): every change writes an `audit_log`
//! row, and a command's rows are consecutive, because a command holds the
//! write lock (IMMEDIATE transaction) from its first write to its commit.
//! The range of those ids is the command's undo token: undoing replays the
//! rows backwards. No extra table is needed, and the history keeps both the
//! command and its undoing.

use std::collections::{BTreeSet, HashMap};

use nm_core::{AuthPolicy, AuthResult, FolderRole, ScreeningTarget, Source, domain_rule_accepts};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde_json::Value;

use crate::{ChangeSet, Result, Store, StoreError, refresh_folder_counts, refresh_thread};

/// Kind of rule a decision is recorded on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleKind {
    Address,
    Domain,
}

impl RuleKind {
    fn table(self) -> &'static str {
        match self {
            RuleKind::Address => "address",
            RuleKind::Domain => "domain",
        }
    }
}

/// A decision taken or changed by a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecisionChange {
    pub kind: RuleKind,
    /// The address or the domain.
    pub name: String,
    pub before: Option<ScreeningTarget>,
    pub after: Option<ScreeningTarget>,
}

/// What a workflow command did, beyond the raw `ChangeSet`.
#[derive(Debug, Default, Clone)]
pub struct Outcome {
    pub changes: ChangeSet,
    /// Messages of the thread the command was given on.
    pub moved: usize,
    /// Other mails filed by the decision.
    pub filed: usize,
    pub decision: Option<DecisionChange>,
    /// Token to pass to `undo`; `None` when nothing changed.
    pub undo: Option<String>,
}

impl DecisionChange {
    /// The contract's view of the decision.
    pub fn view(&self) -> nm_api::DecisionView {
        nm_api::DecisionView { kind: self.kind.table().into(), name: self.name.clone(), before: self.before, after: self.after }
    }
}

impl Outcome {
    /// The contract's view of the outcome.
    pub fn view(&self) -> nm_api::WorkflowResult {
        nm_api::WorkflowResult {
            changed: self.changes.changed_messages(),
            moved: self.moved,
            filed: self.filed,
            decision: self.decision.as_ref().map(DecisionChange::view),
            undo: self.undo.clone(),
        }
    }
}

/// Decision of the domain of `addr`, or of its closest parent domain with
/// one (`news.example.com`, then `example.com`).
pub fn domain_target(conn: &Connection, addr: &str) -> Result<Option<ScreeningTarget>> {
    let Some(domain) = domain_of(addr) else { return Ok(None) };
    let t: Option<String> = conn
        .prepare_cached(
            "SELECT screening_target FROM domain
              WHERE screening_target IS NOT NULL AND (domain = ?1 OR ?1 LIKE '%.' || domain)
              ORDER BY length(domain) DESC LIMIT 1",
        )?
        .query_row([domain], |r| r.get(0))
        .optional()?;
    Ok(t.as_deref().and_then(ScreeningTarget::parse))
}

/// Options of a move, for a thread whose mail is suspected of spoofing.
#[derive(Debug, Default, Clone, Copy)]
pub struct MoveOptions {
    /// The user confirmed a favorable decision on a suspect mail
    /// (`authentification-expediteur.md`, section 5).
    pub confirm_suspect: bool,
    /// Policy chosen in the same confirmation, set on the address.
    pub auth_policy: Option<AuthPolicy>,
}

/// Domain part of an address, lowercase; `None` without `@`.
pub fn domain_of(addr: &str) -> Option<String> {
    addr.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()).filter(|d| !d.is_empty())
}

/// Highest `audit_log` id so far: the rows a command writes come after it.
pub(crate) fn audit_mark(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT coalesce(max(id), 0) FROM audit_log", [], |r| r.get(0))?)
}

/// Undo token of the rows written since `mark`, if any.
pub(crate) fn undo_token(conn: &Connection, mark: i64) -> Result<Option<String>> {
    let last = audit_mark(conn)?;
    Ok((last > mark).then(|| format!("{}:{}", mark + 1, last)))
}

/// Moves one message, clearing its UID, with its `move` audit row. `extra`
/// is merged into the audit detail (the address of a filing, the row being
/// undone). Records the message, its thread and both folders in `changes`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn move_message(
    conn: &Connection,
    message_id: i64,
    thread_id: Option<i64>,
    from_folder: i64,
    to_folder: i64,
    source: Source,
    extra: &Value,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<()> {
    // The IMAP UID is only valid within a server folder: the spool learns
    // the new one when the server moves it (`modele de donnees.md`,
    // section 6). The old one goes to the spool's action first.
    let old_uid: Option<u32> = conn.prepare_cached("SELECT uid FROM message WHERE id = ?1")?.query_row([message_id], |r| r.get(0))?;
    let back = crate::spool::enqueue_move(conn, message_id, from_folder, old_uid, to_folder, source.as_str(), changes)?;
    conn.prepare_cached(
        "UPDATE message SET folder_id = ?2, uid = ?4, date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
    )?
    .execute(params![message_id, to_folder, source.as_str(), back])?;
    // `json_patch` merges `extra` into the before/after object.
    conn.prepare_cached(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, 'message', ?2, 'move',
                 json_patch(json_object('before', json_object('folder_id', ?3),
                                        'after',  json_object('folder_id', ?4)), ?5))",
    )?
    .execute(params![source.as_str(), message_id, from_folder, to_folder, extra.to_string()])?;
    changes.folders.insert(from_folder);
    changes.folders.insert(to_folder);
    changes.messages.insert((message_id, thread_id));
    if let Some(t) = thread_id {
        threads.insert(t);
    }
    Ok(())
}

/// Recomputes the aggregates of the threads and folders a command touched,
/// and lists the threads in `changes`.
pub(crate) fn refresh_all(conn: &Connection, threads: &BTreeSet<i64>, changes: &mut ChangeSet) -> Result<()> {
    for t in threads {
        let folders = refresh_thread(conn, *t)?;
        changes.folders.extend(folders.iter().copied());
        if !changes.threads.iter().any(|(id, _)| id == t) {
            changes.threads.push((*t, folders));
        }
    }
    for f in &changes.folders {
        refresh_folder_counts(conn, *f)?;
    }
    Ok(())
}

/// Current decision of an address or a domain row.
fn current_target(conn: &Connection, kind: RuleKind, id: i64) -> Result<Option<ScreeningTarget>> {
    let sql = format!("SELECT screening_target FROM {} WHERE id = ?1", kind.table());
    let t: Option<String> = conn.prepare_cached(&sql)?.query_row([id], |r| r.get(0))?;
    Ok(t.as_deref().and_then(ScreeningTarget::parse))
}

/// Records `target` on an address or domain row, with its audit row, unless
/// it is already the decision. `trigger`: the message whose move took the
/// decision (`tracabilite.md`: "le message qui a déclenché la décision").
/// Removing a decision keeps a tombstone (NULL target, with a date), so the
/// import never gives the address a decision the user removed. `undo_of`:
/// the audit row this change undoes.
pub(crate) fn record_decision(
    conn: &Connection,
    kind: RuleKind,
    id: i64,
    target: Option<ScreeningTarget>,
    trigger: Option<i64>,
    undo_of: Option<i64>,
    source: Source,
) -> Result<Option<ScreeningTarget>> {
    let before = current_target(conn, kind, id)?;
    if before == target {
        return Ok(before);
    }
    let sql = format!(
        "UPDATE {} SET screening_target = ?2, date_screening_modified = unixepoch(),
                date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
        kind.table()
    );
    conn.prepare_cached(&sql)?.execute(params![id, target.map(|t| t.as_str()), source.as_str()])?;
    conn.prepare_cached(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, ?2, ?3, 'screening',
                 json_object('before', ?4, 'after', ?5, 'message_id', ?6, 'undo_of', ?7))",
    )?
    .execute(params![
        source.as_str(),
        kind.table(),
        id,
        before.map(|t| t.as_str()),
        target.map(|t| t.as_str()),
        trigger,
        undo_of
    ])?;
    Ok(before)
}

/// Sets the authentication policy of an address, with its audit row.
pub(crate) fn record_auth_policy(
    conn: &Connection,
    address_id: i64,
    policy: AuthPolicy,
    undo_of: Option<i64>,
    source: Source,
) -> Result<()> {
    let before: String =
        conn.query_row("SELECT auth_policy FROM address WHERE id = ?1", [address_id], |r| r.get(0))?;
    if before == policy.as_str() {
        return Ok(());
    }
    conn.execute(
        "UPDATE address SET auth_policy = ?2, date_auth_policy_modified = unixepoch(),
                date_modified = unixepoch(), modified_by = ?3 WHERE id = ?1",
        params![address_id, policy.as_str(), source.as_str()],
    )?;
    conn.execute(
        "INSERT INTO audit_log (created_by, entity, entity_id, action, detail)
         VALUES (?1, 'address', ?2, 'auth_policy', json_object('before', ?3, 'after', ?4, 'undo_of', ?5))",
        params![source.as_str(), address_id, before, policy.as_str(), undo_of],
    )?;
    Ok(())
}

/// A mail considered by a filing: id, thread, folder, account, folder
/// role, verdict, flagged, account kind.
type FilingRow = (i64, Option<i64>, i64, i64, String, String, bool, String);

/// Files the mails of one address according to `target`
/// (`ScreeningTarget::filing_sources`). `domain_rule`: the decision is the
/// domain's, which only accepts authenticated mail. Returns the number of
/// mails moved.
///
/// Never moved: flagged mails of the Inbox, and, for a favorable decision,
/// mails whose verdict the rule does not accept (a forged mail stays in
/// Quarantine) except in archive accounts, which have no verdicts worth the
/// name (`import-initial.md`, section 9).
#[allow(clippy::too_many_arguments)]
pub(crate) fn file_address(
    conn: &Connection,
    address_id: i64,
    target: ScreeningTarget,
    domain_rule: bool,
    source: Source,
    folders: &mut HashMap<(i64, FolderRole), Option<i64>>,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<usize> {
    let (addr, policy, auth_seen): (String, String, bool) = conn
        .prepare_cached("SELECT addr, auth_policy, auth_seen FROM address WHERE id = ?1")?
        .query_row([address_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let policy = AuthPolicy::parse(&policy).unwrap_or_default();
    let sources = target.filing_sources();
    // Up to six roles; unused parameters are NULL and match nothing.
    let mut roles: Vec<Option<&str>> = sources.iter().map(|r| Some(r.as_str())).collect();
    roles.resize(6, None);
    // Runs on idx_message_from (from_address_id, folder_id).
    let rows: Vec<FilingRow> = conn
        .prepare_cached(
            "SELECT m.id, m.thread_id, m.folder_id, m.account_id, f.role, m.auth_result, m.is_flagged, ac.kind
               FROM message m JOIN folder f ON f.id = m.folder_id JOIN account ac ON ac.id = m.account_id
              WHERE m.from_address_id = ?1 AND f.role IN (?2, ?3, ?4, ?5, ?6, ?7)",
        )?
        .query_map(params![address_id, roles[0], roles[1], roles[2], roles[3], roles[4], roles[5]], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?))
        })?
        .collect::<std::result::Result<_, _>>()?;
    let extra = serde_json::json!({ if domain_rule { "domain" } else { "address" }: if domain_rule {
        domain_of(&addr).unwrap_or_default()
    } else {
        addr.clone()
    } });
    let mut filed = 0;
    for (id, thread, folder, account, role, verdict, flagged, kind) in rows {
        if role == "inbox" && flagged {
            continue;
        }
        // A read-only account's mail is not filed: the move could not reach
        // its server, and the next synchronization would undo it.
        let read_only: bool = conn
            .prepare_cached("SELECT EXISTS (SELECT 1 FROM temp.read_only_account WHERE id = ?1)")?
            .query_row([account], |r| r.get(0))?;
        if read_only {
            continue;
        }
        if target.is_favorable() && kind != "archive" {
            let verdict = AuthResult::parse(&verdict);
            let accepted = if domain_rule { domain_rule_accepts(verdict) } else { policy.accepts(verdict, auth_seen) };
            if !accepted {
                continue;
            }
        }
        // The target folder of the message's own account; an account
        // without that folder keeps its mail where it is.
        let to = match folders.get(&(account, target.folder())) {
            Some(f) => *f,
            None => {
                let f: Option<i64> = conn
                    .prepare_cached("SELECT id FROM folder WHERE account_id = ?1 AND role = ?2")?
                    .query_row(params![account, target.folder().as_str()], |r| r.get(0))
                    .optional()?;
                folders.insert((account, target.folder()), f);
                f
            }
        };
        let Some(to) = to else { continue };
        move_message(conn, id, thread, folder, to, source, &extra, changes, threads)?;
        filed += 1;
    }
    Ok(filed)
}

/// Files the mails of every address of `domain` that has no decision of
/// its own (`dossiers.md`: "Décision de domaine : même règle, pour les
/// mails des adresses du domaine qui n'ont pas de décision propre"). An
/// address of a subdomain (`news.example.com`) belongs to the domain.
fn file_domain(
    conn: &Connection,
    domain: &str,
    target: ScreeningTarget,
    source: Source,
    changes: &mut ChangeSet,
    threads: &mut BTreeSet<i64>,
) -> Result<usize> {
    let ids: Vec<i64> = conn
        .prepare_cached(
            "SELECT id FROM address
              WHERE screening_target IS NULL
                AND (addr LIKE '%@' || ?1 OR addr LIKE '%@%.' || ?1)",
        )?
        .query_map([domain], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    let mut folders = HashMap::new();
    let mut filed = 0;
    for id in ids {
        filed += file_address(conn, id, target, true, source, &mut folders, changes, threads)?;
    }
    Ok(filed)
}

/// The sender a move decides on: the author of the thread's latest
/// received mail, leaving out the user's own (Sent, Drafts, or from the
/// account's address). The user chose this rule over asking which sender
/// when a thread has several: `dossiers.md`, "Décision sur l'expéditeur".
/// Returns the address id, the address, the id of that mail, and whether
/// a mail of this sender in the thread is suspected of spoofing.
pub(crate) fn thread_sender(conn: &Connection, thread_id: i64) -> Result<Option<(i64, String, i64, bool)>> {
    let row: Option<(i64, String, i64)> = conn
        .prepare_cached(
            "SELECT a.id, a.addr, m.id FROM message m
               JOIN folder f ON f.id = m.folder_id
               JOIN account ac ON ac.id = m.account_id
               JOIN address a ON a.id = m.from_address_id
              WHERE m.thread_id = ?1 AND coalesce(f.role, '') NOT IN ('sent', 'drafts')
                AND (ac.address IS NULL OR a.addr <> ac.address COLLATE NOCASE)
              ORDER BY m.date_received DESC, m.id DESC LIMIT 1",
        )?
        .query_row([thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?;
    let Some((id, addr, message)) = row else { return Ok(None) };
    let suspect: bool = conn
        .prepare_cached("SELECT EXISTS (SELECT 1 FROM message WHERE thread_id = ?1 AND from_address_id = ?2 AND auth_suspect = 1)")?
        .query_row(params![thread_id, id], |r| r.get(0))?;
    Ok(Some((id, addr, message, suspect)))
}

/// Verdict of the sender's suspect mail, for the confirmation message.
pub(crate) fn suspect_verdict(conn: &Connection, thread_id: i64, address_id: i64) -> Result<String> {
    Ok(conn
        .query_row(
            "SELECT auth_result FROM message WHERE thread_id = ?1 AND from_address_id = ?2 AND auth_suspect = 1
              ORDER BY date_received DESC LIMIT 1",
            params![thread_id, address_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_else(|| "unknown".into()))
}

impl Store {
    /// Takes, changes or removes the decision on an address, and files the
    /// mails already received from it (`dossiers.md`, "Décision sur un
    /// expéditeur"). An unknown address is created: a decision can be taken
    /// before its first mail. Removing a decision moves nothing.
    pub fn set_address_screening(&mut self, addr: &str, target: Option<ScreeningTarget>, source: Source) -> Result<Outcome> {
        let addr = addr.trim().to_lowercase();
        if !addr.contains('@') {
            return Err(StoreError::Invalid(nm_api::reason::ADDRESS_INVALID.with("address", addr)));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = audit_mark(&tx)?;
        tx.execute(
            "INSERT INTO address (addr, created_by, modified_by) VALUES (?1, ?2, ?2) ON CONFLICT (addr) DO NOTHING",
            params![addr, source.as_str()],
        )?;
        let id: i64 = tx.query_row("SELECT id FROM address WHERE addr = ?1", [&addr], |r| r.get(0))?;
        let before = record_decision(&tx, RuleKind::Address, id, target, None, None, source)?;
        let mut out = Outcome::default();
        let mut threads = BTreeSet::new();
        if let Some(t) = target
            && source != Source::ConfigSync
        {
            let mut folders = HashMap::new();
            out.filed = file_address(&tx, id, t, false, source, &mut folders, &mut out.changes, &mut threads)?;
        }
        refresh_all(&tx, &threads, &mut out.changes)?;
        out.decision = Some(DecisionChange { kind: RuleKind::Address, name: addr, before, after: target });
        out.undo = undo_token(&tx, mark)?;
        tx.commit()?;
        Ok(out)
    }

    /// Same for a domain: its decision applies to the addresses of the
    /// domain without a decision of their own, and a favorable one only to
    /// authenticated mail.
    pub fn set_domain_screening(&mut self, domain: &str, target: Option<ScreeningTarget>, source: Source) -> Result<Outcome> {
        let domain = domain.trim().trim_start_matches('@').to_lowercase();
        if domain.is_empty() || domain.contains('@') || !domain.contains('.') {
            return Err(StoreError::Invalid(nm_api::reason::DOMAIN_INVALID.with("domain", domain)));
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = audit_mark(&tx)?;
        tx.execute(
            "INSERT INTO domain (domain, created_by, modified_by) VALUES (?1, ?2, ?2) ON CONFLICT (domain) DO NOTHING",
            params![domain, source.as_str()],
        )?;
        let id: i64 = tx.query_row("SELECT id FROM domain WHERE domain = ?1", [&domain], |r| r.get(0))?;
        let before = record_decision(&tx, RuleKind::Domain, id, target, None, None, source)?;
        let mut out = Outcome::default();
        let mut threads = BTreeSet::new();
        if let Some(t) = target
            && source != Source::ConfigSync
        {
            out.filed = file_domain(&tx, &domain, t, source, &mut out.changes, &mut threads)?;
        }
        refresh_all(&tx, &threads, &mut out.changes)?;
        out.decision = Some(DecisionChange { kind: RuleKind::Domain, name: domain, before, after: target });
        out.undo = undo_token(&tx, mark)?;
        tx.commit()?;
        Ok(out)
    }

    /// Undoes a command from its token: each message goes back to the
    /// folder it left, each decision and policy back to its previous value.
    ///
    /// A change made since by someone else wins: a message that is no
    /// longer where the command put it, or a decision changed again, is
    /// left alone. Undoing twice is therefore harmless: the second time
    /// finds nothing in the state the command left.
    pub fn undo(&mut self, token: &str, source: Source) -> Result<Outcome> {
        let bad = || StoreError::InvalidParams(format!("invalid undo token: {token}"));
        let (first, last) = token.split_once(':').ok_or_else(bad)?;
        let (first, last): (i64, i64) = (first.parse().map_err(|_| bad())?, last.parse().map_err(|_| bad())?);
        if first < 1 || last < first {
            return Err(bad());
        }
        let tx = self.conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let mark = audit_mark(&tx)?;
        let rows: Vec<(i64, String, i64, String, String)> = tx
            .prepare(
                "SELECT id, entity, entity_id, action, detail FROM audit_log
                  WHERE id BETWEEN ?1 AND ?2 AND action IN ('move', 'screening', 'auth_policy', 'sender_kind')
                  ORDER BY id DESC",
            )?
            .query_map(params![first, last], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<std::result::Result<_, _>>()?;
        if rows.is_empty() {
            return Err(StoreError::NotFound(format!("operation {token}")));
        }
        let mut out = Outcome::default();
        let mut threads = BTreeSet::new();
        for (row, entity, id, action, detail) in rows {
            let detail: Value = serde_json::from_str(&detail).unwrap_or(Value::Null);
            let undo_of = serde_json::json!({ "undo_of": row });
            match (entity.as_str(), action.as_str()) {
                ("message", "move") => {
                    let (Some(before), Some(after)) =
                        (detail["before"]["folder_id"].as_i64(), detail["after"]["folder_id"].as_i64())
                    else {
                        continue;
                    };
                    let now: Option<(i64, Option<i64>)> = tx
                        .query_row("SELECT folder_id, thread_id FROM message WHERE id = ?1", [id], |r| {
                            Ok((r.get(0)?, r.get(1)?))
                        })
                        .optional()?;
                    let folder_exists: bool =
                        tx.query_row("SELECT EXISTS (SELECT 1 FROM folder WHERE id = ?1)", [before], |r| r.get(0))?;
                    if let Some((current, thread)) = now
                        && current == after
                        && folder_exists
                    {
                        move_message(&tx, id, thread, after, before, source, &undo_of, &mut out.changes, &mut threads)?;
                        out.moved += 1;
                    }
                }
                ("address" | "domain", "screening") => {
                    let kind = if entity == "address" { RuleKind::Address } else { RuleKind::Domain };
                    let parse = |v: &Value| v.as_str().and_then(ScreeningTarget::parse);
                    if current_target(&tx, kind, id)? == parse(&detail["after"]) {
                        record_decision(&tx, kind, id, parse(&detail["before"]), None, Some(row), source)?;
                    }
                }
                ("address", "auth_policy") => {
                    let current: String =
                        tx.query_row("SELECT auth_policy FROM address WHERE id = ?1", [id], |r| r.get(0))?;
                    if Some(current.as_str()) == detail["after"].as_str()
                        && let Some(p) = detail["before"].as_str().and_then(AuthPolicy::parse)
                    {
                        record_auth_policy(&tx, id, p, Some(row), source)?;
                    }
                }
                // The verdict forced on an address (`expediteurs.md`,
                // section 9): back to the previous choice, or to the
                // computation, if nobody changed it since.
                ("address", "sender_kind") => {
                    let current: Option<String> =
                        tx.query_row("SELECT sender_kind_user FROM address WHERE id = ?1", [id], |r| r.get(0))?;
                    if current.as_deref() == detail["after"].as_str() {
                        let before = detail["before"].as_str().and_then(nm_core::SenderKind::parse);
                        crate::senders::force_sender_kind(&tx, id, before, Some(row), source)?;
                    }
                }
                _ => {}
            }
        }
        refresh_all(&tx, &threads, &mut out.changes)?;
        out.undo = undo_token(&tx, mark)?;
        tx.commit()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_of_lowercases_and_ignores_garbage() {
        assert_eq!(domain_of("Jean@Exemple.FR").as_deref(), Some("exemple.fr"));
        assert_eq!(domain_of("x@a@b.org").as_deref(), Some("b.org"));
        assert_eq!(domain_of("nobody"), None);
        assert_eq!(domain_of("x@"), None);
    }
}
