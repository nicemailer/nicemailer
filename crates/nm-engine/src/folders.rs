//! Which server folder could hold which workflow role (`dossiers.md`, "A la
//! création du compte"): the proposal the user confirms, shown by
//! `nicemailer imap probe` now, and by the account creation panel later
//! (phase 7).
//!
//! The rules, in order:
//! 1. INBOX is the Inbox.
//! 2. A special-use attribute (RFC 6154) the server declares gives the
//!    role: `\Sent`, `\Drafts`, `\Trash` (Deleted), `\Junk`, `\Archive`. A
//!    folder the phone already uses as Drafts is taken over as is, and
//!    NiceMailer creates no second one.
//! 3. Failing that, the name: the usual names in English and French, and
//!    the names NiceMailer gives its own folders (`Todo`, `Waiting`…).
//!    Only a folder at the top of the personal namespace counts:
//!    `Clients/Sent` is a folder of the user, not the Sent folder.
//!
//! Each role goes to one folder at most: the attribute wins over the name,
//! then the first name of the list, then the first folder the server
//! listed. `\All` and `\Flagged` are virtual views of other folders: never
//! a role.

use nm_proto::imap::{ListEntry, Namespace, SpecialUse};

/// Why a role is proposed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleFrom {
    Inbox,
    SpecialUse,
    Name,
}

impl RoleFrom {
    pub fn as_str(self) -> &'static str {
        match self {
            RoleFrom::Inbox => "inbox",
            RoleFrom::SpecialUse => "special-use",
            RoleFrom::Name => "name",
        }
    }
}

/// Usual names of each role, lowercase, by preference. The roles are those
/// of `folder.role` (`schema.sql`).
const NAMES: &[(&str, &[&str])] = &[
    ("sent", &["sent", "sent items", "sent messages", "sent mail", "envoyés", "messages envoyés", "éléments envoyés"]),
    ("drafts", &["drafts", "brouillons"]),
    ("deleted", &["trash", "deleted", "deleted items", "deleted messages", "corbeille", "éléments supprimés"]),
    ("junk", &["junk", "spam", "junk e-mail", "junk email", "courrier indésirable", "indésirables", "pourriels"]),
    ("archive", &["archive", "archives"]),
    ("todo", &["todo"]),
    ("waiting", &["waiting"]),
    ("newsletter", &["newsletter", "newsletters"]),
    ("paperwork", &["paperwork"]),
    ("quarantine", &["quarantine"]),
    ("banned", &["banned"]),
];

fn role_of_use(u: SpecialUse) -> Option<&'static str> {
    match u {
        SpecialUse::Sent => Some("sent"),
        SpecialUse::Drafts => Some("drafts"),
        SpecialUse::Trash => Some("deleted"),
        SpecialUse::Junk => Some("junk"),
        SpecialUse::Archive => Some("archive"),
        SpecialUse::All | SpecialUse::Flagged => None,
    }
}

/// The role proposed for each folder of `entries` (same order), with the
/// reason. `personal`: the personal namespace, whose prefix (`INBOX.` on
/// some servers) is not part of the name looked at.
pub fn propose_roles(entries: &[ListEntry], personal: Option<&Namespace>) -> Vec<Option<(&'static str, RoleFrom)>> {
    let mut out: Vec<Option<(&'static str, RoleFrom)>> = vec![None; entries.len()];
    let taken = |out: &Vec<Option<(&'static str, RoleFrom)>>, role: &str| out.iter().flatten().any(|(r, _)| *r == role);

    for (i, e) in entries.iter().enumerate() {
        if e.name.is_inbox() {
            out[i] = Some(("inbox", RoleFrom::Inbox));
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if out[i].is_some() || !e.selectable() {
            continue;
        }
        if let Some(role) = e.special_use().and_then(role_of_use)
            && !taken(&out, role)
        {
            out[i] = Some((role, RoleFrom::SpecialUse));
        }
    }
    for (role, names) in NAMES {
        if taken(&out, role) {
            continue;
        }
        // Rust note: `'found:` labels the outer loop, so that `break 'found`
        // leaves both loops at once.
        'found: for name in *names {
            for (i, e) in entries.iter().enumerate() {
                if out[i].is_none() && e.selectable() && top_level_name(e, personal).is_some_and(|n| n.to_lowercase() == *name) {
                    out[i] = Some((role, RoleFrom::Name));
                    break 'found;
                }
            }
        }
    }
    out
}

/// The name of a folder at the top of the personal namespace, without the
/// prefix; `None` for a folder deeper in the hierarchy. `INBOX/Sent` also
/// counts as top level: some servers put every folder under INBOX without
/// saying so in NAMESPACE.
fn top_level_name<'a>(e: &'a ListEntry, personal: Option<&Namespace>) -> Option<&'a str> {
    let mut name = e.name.name.as_str();
    if let Some(prefix) = personal.map(|n| n.prefix.as_str()).filter(|p| !p.is_empty()) {
        name = name.strip_prefix(prefix).unwrap_or(name);
    }
    let Some(d) = e.delimiter else { return Some(name) };
    if let Some((first, rest)) = name.split_once(d)
        && first.eq_ignore_ascii_case("INBOX")
    {
        name = rest;
    }
    (!name.contains(d)).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nm_proto::imap::MailboxName;

    fn entry(name: &str, attrs: &[&str]) -> ListEntry {
        ListEntry { name: MailboxName::new(name), delimiter: Some('/'), attributes: attrs.iter().map(|a| a.to_string()).collect() }
    }

    fn roles(entries: &[ListEntry], ns: Option<&Namespace>) -> Vec<Option<&'static str>> {
        propose_roles(entries, ns).into_iter().map(|r| r.map(|(role, _)| role)).collect()
    }

    #[test]
    fn special_use_wins_over_names() {
        let e = [
            entry("INBOX", &[]),
            entry("Sent", &[]),
            entry("Envoyés", &["\\Sent"]),
            entry("Trash", &["\\Trash"]),
            entry("All Mail", &["\\All"]),
            entry("Starred", &["\\Flagged"]),
        ];
        let r = propose_roles(&e, None);
        assert_eq!(r[0], Some(("inbox", RoleFrom::Inbox)));
        assert_eq!(r[1], None, "the attribute of another folder wins");
        assert_eq!(r[2], Some(("sent", RoleFrom::SpecialUse)));
        assert_eq!(r[3], Some(("deleted", RoleFrom::SpecialUse)));
        assert_eq!(r[4], None, "\\All is not a role");
        assert_eq!(r[5], None);
    }

    #[test]
    fn names_in_english_and_french_at_top_level_only() {
        let e = [
            entry("INBOX", &[]),
            entry("Éléments envoyés", &[]),
            entry("Corbeille", &[]),
            entry("Courrier indésirable", &[]),
            entry("Clients/Archive", &[]),
            entry("Brouillons", &[]),
            entry("Todo", &[]),
            entry("Quarantine", &[]),
        ];
        assert_eq!(roles(&e, None), vec![
            Some("inbox"),
            Some("sent"),
            Some("deleted"),
            Some("junk"),
            None,
            Some("drafts"),
            Some("todo"),
            Some("quarantine"),
        ]);
    }

    #[test]
    fn one_folder_per_role_preferred_name_first() {
        let e = [entry("Spam", &[]), entry("Junk", &[]), entry("Archives", &[]), entry("Archive", &[])];
        assert_eq!(roles(&e, None), vec![None, Some("junk"), None, Some("archive")]);
    }

    #[test]
    fn namespace_prefix_and_inbox_parent() {
        let ns = Namespace { prefix: "INBOX/".into(), delimiter: Some('/') };
        let e = [entry("INBOX", &[]), entry("INBOX/Sent", &[]), entry("INBOX/Clients/Trash", &[])];
        assert_eq!(roles(&e, Some(&ns)), vec![Some("inbox"), Some("sent"), None]);
        // INBOX/Drafts counts without NAMESPACE too.
        assert_eq!(roles(&[entry("INBOX/Drafts", &[])], None), vec![Some("drafts")]);
    }

    #[test]
    fn folders_that_cannot_hold_mail_get_no_role() {
        let e = [entry("Sent", &["\\Noselect"]), entry("Trash", &["\\NonExistent", "\\Trash"])];
        assert_eq!(roles(&e, None), vec![None, None]);
    }
}
