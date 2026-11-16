//! Pure domain logic of NiceMailer: folder roles and workflow rules.
//!
//! This crate performs no I/O at all (no database, no network, no files).
//! It only encodes the rules described in `specifications/dossiers.md`, so
//! that every other crate (store, daemon, CLI) shares the same definitions.
//! Keeping it pure also makes it trivial to unit test.
//!
//! Rust note: `//!` comments document the enclosing item (here the whole
//! crate), while `///` comments document the item that follows them. Both
//! are rendered by `cargo doc`.

pub mod compose;
pub mod formats;
pub mod segments;

use serde::{Deserialize, Serialize};

/// Role of a folder in the workflow (the `folder.role` column).
///
/// Rust note: `#[derive(...)]` asks the compiler to generate standard trait
/// implementations for us:
/// - `Debug` allows printing with `{:?}`,
/// - `Clone, Copy` let the value be duplicated implicitly (it is just a
///   small integer under the hood, so copying is free),
/// - `PartialEq, Eq, Hash` allow comparisons and use as a `HashMap` key,
/// - `Serialize, Deserialize` come from the `serde` library and convert the
///   value to and from JSON.
///
/// `#[serde(rename_all = "snake_case")]` makes `FolderRole::Inbox` appear as
/// `"inbox"` in JSON instead of `"Inbox"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderRole {
    Inbox,
    Todo,
    Waiting,
    Archive,
    Sent,
    Drafts,
    Newsletter,
    Paperwork,
    Quarantine,
    Deleted,
    Junk,
    Banned,
}

/// Category of a folder (`dossiers.md`, "Catégories de dossiers").
/// The app uses it to group folders in the sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderCategory {
    /// Folders where the user processes mail: Inbox, Todo, Waiting.
    Work,
    /// Folders where mail is filed once processed.
    Filing,
    /// Folders for unwanted mail.
    Rejection,
    /// Folders managed by the system rather than by the user's decisions.
    Technical,
}

// Rust note: an `impl` block attaches functions ("methods") to a type.
// Methods taking `self` are called as `role.as_str()`; functions without
// `self` (like `parse`) are called as `FolderRole::parse("inbox")`.
impl FolderRole {
    /// Every role, in sidebar display order.
    ///
    /// Rust note: a `const` is evaluated at compile time. `[FolderRole; 12]`
    /// is a fixed-size array of exactly 12 elements; the compiler checks the
    /// count.
    pub const ALL: [FolderRole; 12] = [
        FolderRole::Inbox,
        FolderRole::Todo,
        FolderRole::Waiting,
        FolderRole::Archive,
        FolderRole::Sent,
        FolderRole::Drafts,
        FolderRole::Newsletter,
        FolderRole::Paperwork,
        FolderRole::Quarantine,
        FolderRole::Deleted,
        FolderRole::Junk,
        FolderRole::Banned,
    ];

    /// Folders the virtual folder "All Mail" leaves out (`ui.md`, "Sélecteur
    /// d'attributs"): what the user deleted or rejected. Searches mostly
    /// want everything else, Quarantine included (the mail of a sender not
    /// accepted yet is searched for too). "Everything" keeps them.
    pub const OUT_OF_ALL_MAIL: [FolderRole; 3] =
        [FolderRole::Deleted, FolderRole::Junk, FolderRole::Banned];

    /// The folders of the virtual folder "Work" (`ui.md`, "Dossiers
    /// virtuels"): what is still to deal with.
    pub const WORK: [FolderRole; 3] = [FolderRole::Inbox, FolderRole::Todo, FolderRole::Waiting];

    /// Folders whose messages follow a command applied to the conversation
    /// (`dossiers.md`, "Une conversation, pas un mail").
    ///
    /// Example: archiving a thread moves its messages that sit in Inbox or
    /// Todo, but never the copies in Sent, nor a message the user explicitly
    /// put in Deleted.
    pub const CONVERSATION_SCOPE: [FolderRole; 6] = [
        FolderRole::Inbox,
        FolderRole::Todo,
        FolderRole::Waiting,
        FolderRole::Archive,
        FolderRole::Newsletter,
        FolderRole::Paperwork,
    ];

    /// Identifier stored in the database and sent over the API.
    ///
    /// Rust note: `&'static str` is a borrowed string that lives for the
    /// whole program (string literals are embedded in the binary), so no
    /// memory allocation is needed. `match` must cover every variant: if a
    /// role is added later, the compiler will point at every `match` to
    /// update.
    pub fn as_str(self) -> &'static str {
        match self {
            FolderRole::Inbox => "inbox",
            FolderRole::Todo => "todo",
            FolderRole::Waiting => "waiting",
            FolderRole::Archive => "archive",
            FolderRole::Sent => "sent",
            FolderRole::Drafts => "drafts",
            FolderRole::Newsletter => "newsletter",
            FolderRole::Paperwork => "paperwork",
            FolderRole::Quarantine => "quarantine",
            FolderRole::Deleted => "deleted",
            FolderRole::Junk => "junk",
            FolderRole::Banned => "banned",
        }
    }

    /// Inverse of [`FolderRole::as_str`].
    ///
    /// Rust note: Rust has no `null`. A value that may be absent is an
    /// `Option<T>`, which is either `Some(value)` or `None`, and the caller
    /// is forced to handle both cases. `find` returns the first element for
    /// which the closure `|r| ...` returns true.
    pub fn parse(s: &str) -> Option<FolderRole> {
        FolderRole::ALL.into_iter().find(|r| r.as_str() == s)
    }

    /// One-letter code of the role in `thread.folder` (`schema.sql`, version
    /// 9): the lower-case letter of [`FolderRole::letter`]. A single letter
    /// rather than the name keeps the column and its indexes small; it is the
    /// same letter as the move shortcut, so it reads easily in SQL.
    pub fn code(self) -> &'static str {
        match self {
            FolderRole::Inbox => "i",
            FolderRole::Todo => "t",
            FolderRole::Waiting => "w",
            FolderRole::Archive => "a",
            FolderRole::Sent => "s",
            FolderRole::Drafts => "r",
            FolderRole::Newsletter => "n",
            FolderRole::Paperwork => "p",
            FolderRole::Quarantine => "q",
            FolderRole::Deleted => "d",
            FolderRole::Junk => "x",
            FolderRole::Banned => "b",
        }
    }

    /// Inverse of [`FolderRole::code`].
    pub fn from_code(s: &str) -> Option<FolderRole> {
        FolderRole::ALL.into_iter().find(|r| r.code() == s)
    }

    /// Technical folders (`dossiers.md`, "Dossier d'une conversation"):
    /// they hold some mails of a conversation for technical reasons (the
    /// mail sent, a draft, the mail of an unknown sender, a mail deleted
    /// alone), never what the conversation is. A conversation has a flag
    /// for each (`thread.has_sent`…), in this order, which is also the
    /// fixed order of the fallback of [`thread_folder`].
    pub const TECHNICAL: [FolderRole; 4] = [
        FolderRole::Sent,
        FolderRole::Drafts,
        FolderRole::Quarantine,
        FolderRole::Deleted,
    ];

    /// Whether this is one of [`FolderRole::TECHNICAL`].
    pub fn is_technical(self) -> bool {
        FolderRole::TECHNICAL.contains(&self)
    }

    /// Default display name. The user will later be able to rename folders,
    /// which is why the role and the name are kept separate.
    pub fn display_name(self) -> &'static str {
        match self {
            FolderRole::Inbox => "Inbox",
            FolderRole::Todo => "Todo",
            FolderRole::Waiting => "Waiting",
            FolderRole::Archive => "Archive",
            FolderRole::Sent => "Sent",
            FolderRole::Drafts => "Drafts",
            FolderRole::Newsletter => "Newsletter",
            FolderRole::Paperwork => "Paperwork",
            FolderRole::Quarantine => "Quarantine",
            FolderRole::Deleted => "Deleted",
            FolderRole::Junk => "Junk",
            FolderRole::Banned => "Banned",
        }
    }

    /// Keyboard shortcut letter used to move a thread to this folder.
    /// H, J, K and L are reserved for vim-style navigation, which is why
    /// Drafts uses R and Junk uses X.
    pub fn letter(self) -> char {
        match self {
            FolderRole::Inbox => 'I',
            FolderRole::Todo => 'T',
            FolderRole::Waiting => 'W',
            FolderRole::Archive => 'A',
            FolderRole::Sent => 'S',
            FolderRole::Drafts => 'R',
            FolderRole::Newsletter => 'N',
            FolderRole::Paperwork => 'P',
            FolderRole::Quarantine => 'Q',
            FolderRole::Deleted => 'D',
            FolderRole::Junk => 'X',
            FolderRole::Banned => 'B',
        }
    }

    /// Category this folder belongs to.
    pub fn category(self) -> FolderCategory {
        // `use FolderRole::*` brings the variants into scope so we can write
        // `Inbox` instead of `FolderRole::Inbox`. The `|` in a match arm means
        // "any of these".
        use FolderRole::*;
        match self {
            Inbox | Todo | Waiting => FolderCategory::Work,
            Archive | Newsletter | Paperwork => FolderCategory::Filing,
            Junk | Banned => FolderCategory::Rejection,
            Sent | Drafts | Quarantine | Deleted => FolderCategory::Technical,
        }
    }

    /// Targets offered by the move command when the user is in this folder
    /// (`dossiers.md`, "Cibles proposées").
    ///
    /// - Sent offers the same targets as the work and filing folders: a
    ///   conversation listed in Sent can be filed from there, and a new mail
    ///   with no answer yet can go to Waiting or Todo (added in November
    ///   2026). The store decides which mails follow (`apply_move`).
    /// - Drafts offers Deleted only: a draft is either sent or deleted.
    /// - Quarantine is a screening step: from there the user can accept the
    ///   sender (Inbox, Newsletter, Paperwork) or reject them. Todo and
    ///   Waiting are not offered, because the sender must be accepted first.
    /// - Every other folder offers all work and filing folders except itself.
    ///
    /// Quarantine itself is never a target: only the core puts mail there.
    ///
    /// Rust note: `Vec<T>` is a growable, heap-allocated list. `vec![]` builds
    /// one. The last arm builds an array, turns it into an iterator, removes
    /// the current folder with `filter`, and `collect`s the rest into a `Vec`.
    pub fn move_targets(self) -> Vec<FolderRole> {
        use FolderRole::*;
        match self {
            // A draft only goes to Deleted (`del`): a draft of another
            // client (Spark, the phone), which NiceMailer cannot open, could
            // not be removed otherwise (`dossiers.md`, "Sent et Drafts").
            Drafts => vec![Deleted],
            Quarantine => vec![Inbox, Newsletter, Paperwork, Junk, Banned, Deleted],
            // `_` matches every case not handled above.
            _ => [
                Todo, Waiting, Inbox, Archive, Newsletter, Paperwork, Junk, Banned, Deleted,
            ]
            .into_iter()
            .filter(|r| *r != self)
            .collect(),
        }
    }

    /// Folder preselected when the user replies to a thread that sits in
    /// this folder (`dossiers.md`, "Répondre à une conversation").
    ///
    /// `returned_from` tells whether the thread came back to the Inbox from
    /// Todo or Waiting because a new message arrived. In that case, replying
    /// sends it back where it came from: the user is still waiting for an
    /// answer, or still has something to do.
    ///
    /// Returns `None` when no filing choice should be offered.
    pub fn reply_target(self, returned_from: Option<FolderRole>) -> Option<FolderRole> {
        use FolderRole::*;
        match self {
            // A `match` is an expression: its value is wrapped in `Some(...)`.
            Inbox => Some(match returned_from {
                Some(Waiting) => Waiting,
                Some(Todo) => Todo,
                _ => Archive,
            }),
            Todo => Some(Archive),
            Waiting => Some(Waiting),
            _ => None,
        }
    }
}

/// What becomes of the conversation once a reply is sent, when the user
/// chose it (`⇧⌘↩`, `reponse.md`, section 8). Without a choice the core
/// applies its preselection ([`FolderRole::reply_target`]).
///
/// The three filing targets of `dossiers.md` ("Répondre à une
/// conversation"), and `Stay`, a trial option: the conversation is left
/// where it is, as if the reply had not been sent (nothing moved, the
/// return marks kept). It serves the reply that answers a question but
/// leaves the conversation to be handled in the Inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AfterSend {
    Archive,
    Waiting,
    Todo,
    Stay,
}

impl AfterSend {
    pub const ALL: [AfterSend; 4] = [
        AfterSend::Archive,
        AfterSend::Waiting,
        AfterSend::Todo,
        AfterSend::Stay,
    ];

    /// Identifier stored in the database (`draft.after_send`) and in the
    /// payload of a `send` action.
    pub fn as_str(self) -> &'static str {
        match self {
            AfterSend::Archive => "archive",
            AfterSend::Waiting => "waiting",
            AfterSend::Todo => "todo",
            AfterSend::Stay => "stay",
        }
    }

    pub fn parse(s: &str) -> Option<AfterSend> {
        AfterSend::ALL.into_iter().find(|a| a.as_str() == s)
    }

    /// The folder the conversation goes to; `None` for `Stay`.
    pub fn folder(self) -> Option<FolderRole> {
        match self {
            AfterSend::Archive => Some(FolderRole::Archive),
            AfterSend::Waiting => Some(FolderRole::Waiting),
            AfterSend::Todo => Some(FolderRole::Todo),
            AfterSend::Stay => None,
        }
    }
}

/// Who writes the mails of an address (`expediteurs.md`): a person or a
/// machine. Stored in `address.sender_kind` (computed),
/// `address.sender_kind_user` (forced by the user) and `message.sender_kind`
/// (what the list filter reads).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderKind {
    Person,
    Robot,
}

impl SenderKind {
    /// Identifier stored in the database and sent to the clients.
    pub fn as_str(self) -> &'static str {
        match self {
            SenderKind::Person => "person",
            SenderKind::Robot => "robot",
        }
    }

    pub fn parse(s: &str) -> Option<SenderKind> {
        [SenderKind::Person, SenderKind::Robot]
            .into_iter()
            .find(|k| k.as_str() == s)
    }
}

/// Screening decision on an address or a domain (`address.screening_target`,
/// `domain.screening_target`): where the next mails of that sender go.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreeningTarget {
    Inbox,
    Newsletter,
    Paperwork,
    Banned,
}

impl ScreeningTarget {
    pub const ALL: [ScreeningTarget; 4] = [
        ScreeningTarget::Inbox,
        ScreeningTarget::Newsletter,
        ScreeningTarget::Paperwork,
        ScreeningTarget::Banned,
    ];

    /// Identifier stored in the database.
    pub fn as_str(self) -> &'static str {
        self.folder().as_str()
    }

    pub fn parse(s: &str) -> Option<ScreeningTarget> {
        ScreeningTarget::ALL.into_iter().find(|t| t.as_str() == s)
    }

    /// The folder the decision files mail into.
    pub fn folder(self) -> FolderRole {
        match self {
            ScreeningTarget::Inbox => FolderRole::Inbox,
            ScreeningTarget::Newsletter => FolderRole::Newsletter,
            ScreeningTarget::Paperwork => FolderRole::Paperwork,
            ScreeningTarget::Banned => FolderRole::Banned,
        }
    }

    /// A favorable decision lets the sender's mail in; it only applies to
    /// authenticated mail (`authentification-expediteur.md`, section 4).
    /// Banned applies whatever the verdict: refusing a forged mail too is
    /// not a risk.
    pub fn is_favorable(self) -> bool {
        self != ScreeningTarget::Banned
    }

    /// Folders whose mails of the sender are filed when the user takes this
    /// decision (`dossiers.md`, "Décision sur un expéditeur : rangement des
    /// autres mails").
    ///
    /// - Inbox only empties the Quarantine: mail already archived has been
    ///   dealt with, bringing years of it back to the Inbox would undo work.
    /// - The others also take the sender's mail from the Inbox, the Archive
    ///   and the two other decision folders.
    ///
    /// Todo, Waiting, Sent, Drafts, Deleted and Junk never appear: they
    /// result from an explicit command of the user.
    pub fn filing_sources(self) -> Vec<FolderRole> {
        use FolderRole::*;
        match self {
            ScreeningTarget::Inbox => vec![Quarantine],
            _ => [Quarantine, Inbox, Archive, Newsletter, Paperwork, Banned]
                .into_iter()
                .filter(|r| *r != self.folder())
                .collect(),
        }
    }

    /// Decision recorded on the sender when the user moves a thread from
    /// `from` to `to` (`dossiers.md`, "Déplacement et décision sur
    /// l'expéditeur"). `from` is `None` when the client gave no folder.
    ///
    /// - Newsletter, Paperwork, Banned: from any folder. If moving a
    ///   newsletter out of the Inbox did not decide, the next one would come
    ///   back to the Inbox.
    /// - Inbox: only out of Quarantine or Banned, where it accepts a sender.
    ///   Elsewhere (back from Archive, say) it is about this mail only.
    /// - Junk never decides: spam changes its sending address every time.
    /// - Todo, Waiting, Archive, Deleted: this mail only.
    pub fn for_move(from: Option<FolderRole>, to: FolderRole) -> Option<ScreeningTarget> {
        use FolderRole::*;
        match to {
            Newsletter => Some(ScreeningTarget::Newsletter),
            Paperwork => Some(ScreeningTarget::Paperwork),
            Banned => Some(ScreeningTarget::Banned),
            Inbox if matches!(from, Some(Quarantine) | Some(Banned)) => {
                Some(ScreeningTarget::Inbox)
            }
            _ => None,
        }
    }
}

/// Authentication verdict of a received message (`message.auth_result`,
/// `authentification-expediteur.md`, section 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthResult {
    Pass,
    Fail,
    None,
    Unknown,
}

impl AuthResult {
    /// Unknown strings read as `Unknown`, the verdict that proves nothing.
    pub fn parse(s: &str) -> AuthResult {
        match s {
            "pass" => AuthResult::Pass,
            "fail" => AuthResult::Fail,
            "none" => AuthResult::None,
            _ => AuthResult::Unknown,
        }
    }

    /// The value stored in `message.auth_result`.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthResult::Pass => "pass",
            AuthResult::Fail => "fail",
            AuthResult::None => "none",
            AuthResult::Unknown => "unknown",
        }
    }
}

/// The user's authentication policy for an address (`address.auth_policy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthPolicy {
    /// `pass` required once a `pass` mail has been seen from the address.
    #[default]
    Auto,
    /// Always `pass` (bank, administration).
    Required,
    /// Any verdict, `fail` included (a legitimate sender using a badly
    /// configured tool).
    NotRequired,
}

impl AuthPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthPolicy::Auto => "auto",
            AuthPolicy::Required => "required",
            AuthPolicy::NotRequired => "not_required",
        }
    }

    pub fn parse(s: &str) -> Option<AuthPolicy> {
        [
            AuthPolicy::Auto,
            AuthPolicy::Required,
            AuthPolicy::NotRequired,
        ]
        .into_iter()
        .find(|p| p.as_str() == s)
    }

    /// Whether a favorable address rule applies to a mail with `verdict`
    /// (`authentification-expediteur.md`, "Politique d'authentification par
    /// adresse"). `auth_seen`: a `pass` mail has already been received from
    /// the address, so an unauthenticated one is now suspicious.
    pub fn accepts(self, verdict: AuthResult, auth_seen: bool) -> bool {
        match self {
            AuthPolicy::NotRequired => true,
            AuthPolicy::Required => verdict == AuthResult::Pass,
            AuthPolicy::Auto => match verdict {
                AuthResult::Pass => true,
                AuthResult::None | AuthResult::Unknown => !auth_seen,
                AuthResult::Fail => false,
            },
        }
    }
}

/// Whether a favorable *domain* rule applies: the domain has no policy and
/// always requires `pass` (`authentification-expediteur.md`, section 4).
pub fn domain_rule_accepts(verdict: AuthResult) -> bool {
    verdict == AuthResult::Pass
}

/// Origin of a creation or a modification (`tracabilite.md`). It is stored
/// in `created_by` / `modified_by` columns and in the audit log, so that one
/// can later tell whether a change came from the app, the CLI, IMAP, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Core,
    App,
    Cli,
    Tui,
    Web,
    Mcp,
    ConfigSync,
    Imap,
}

impl Source {
    /// The source of an identifier stored in the database; `None` for an
    /// unknown one.
    pub fn parse(s: &str) -> Option<Source> {
        Some(match s {
            "core" => Source::Core,
            "app" => Source::App,
            "cli" => Source::Cli,
            "tui" => Source::Tui,
            "web" => Source::Web,
            "mcp" => Source::Mcp,
            "config_sync" => Source::ConfigSync,
            "imap" => Source::Imap,
            _ => return None,
        })
    }

    /// Identifier stored in the database.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Core => "core",
            Source::App => "app",
            Source::Cli => "cli",
            Source::Tui => "tui",
            Source::Web => "web",
            Source::Mcp => "mcp",
            Source::ConfigSync => "config_sync",
            Source::Imap => "imap",
        }
    }
}

/// IMAP keywords reserved to the core. They mark a thread that came back to
/// the Inbox from Todo or Waiting, so the UI can show where it came from and
/// `reply_target` can send it back there. Any command on the thread clears
/// them. Storing them as IMAP keywords means the mark survives a resync
/// with the server.
pub const KW_WAS_TODO: &str = "$NmWasTodo";
pub const KW_WAS_WAITING: &str = "$NmWasWaiting";

/// The folder of a conversation (`dossiers.md`, "Dossier d'une
/// conversation"), from the roles of the folders of its mails, **latest mail
/// first**; `None` for a mail in a folder without a role. The first case
/// that applies:
/// 1. the folder of the latest mail in Inbox, Todo, Waiting, Archive,
///    Newsletter or Paperwork;
/// 2. else that of the latest mail in Junk or Banned: a command on the
///    conversation never touches those mails, so a rejected mail left there
///    does not say what the conversation is;
/// 3. else Sent, Drafts, Quarantine, Deleted, the first that holds a mail
///    (fixed order: Deleted, the only "not alive" one, comes last);
/// 4. else `None`: only folders without a role.
///
/// A pure function so that the rule is tested on its own; the store feeds
/// it the mails of the thread (`refresh_thread`), and the migration to
/// schema version 9 writes the same rule in SQL (`migrations.rs`, tested
/// against this function).
///
/// Rust note: `impl IntoIterator<Item = ...>` accepts any sequence (a
/// `Vec`, an array, an iterator), read once; `Clone` lets it be walked
/// again for each case.
pub fn thread_folder<I>(roles_latest_first: I) -> Option<FolderRole>
where
    I: IntoIterator<Item = Option<FolderRole>>,
    I::IntoIter: Clone,
{
    let roles = roles_latest_first.into_iter();
    use FolderRole::*;
    let live =
        |r: &FolderRole| matches!(r, Inbox | Todo | Waiting | Archive | Newsletter | Paperwork);
    let rejected = |r: &FolderRole| matches!(r, Junk | Banned);
    // `flatten` skips the `None` (no role); `find` stops at the first,
    // that is the latest, mail of the case.
    if let Some(r) = roles.clone().flatten().find(live) {
        return Some(r);
    }
    if let Some(r) = roles.clone().flatten().find(rejected) {
        return Some(r);
    }
    FolderRole::TECHNICAL
        .into_iter()
        .find(|t| roles.clone().flatten().any(|r| r == *t))
}

// Rust note: `#[cfg(test)]` means this module is only compiled when running
// `cargo test`. Each function marked `#[test]` is a test case; it fails if
// it panics, which is what `assert!` and `assert_eq!` do on failure.
#[cfg(test)]
mod tests {
    // Import everything from the parent module (this file).
    use super::*;

    #[test]
    fn letters_are_unique_and_avoid_vim_keys() {
        let mut letters: Vec<char> = FolderRole::ALL.iter().map(|r| r.letter()).collect();
        assert!(!letters.iter().any(|c| "HJKL".contains(*c)));
        // After sorting and removing consecutive duplicates, the length is
        // unchanged only if every letter was unique.
        letters.sort();
        letters.dedup();
        assert_eq!(letters.len(), FolderRole::ALL.len());
    }

    #[test]
    fn codes_are_unique_and_parse_back() {
        for r in FolderRole::ALL {
            assert_eq!(FolderRole::from_code(r.code()), Some(r));
            assert_eq!(r.code(), r.letter().to_ascii_lowercase().to_string());
        }
        assert_eq!(FolderRole::from_code("z"), None);
    }

    #[test]
    fn the_folder_of_a_thread_follows_the_spec() {
        use FolderRole::*;
        let f = |roles: &[Option<FolderRole>]| thread_folder(roles.iter().copied());
        // 1. The latest mail in a real folder, technical ones skipped.
        assert_eq!(f(&[Some(Sent), Some(Inbox), Some(Archive)]), Some(Inbox));
        assert_eq!(f(&[Some(Archive), Some(Inbox)]), Some(Archive));
        // A new mail brings the whole conversation back to the Inbox.
        assert_eq!(f(&[Some(Inbox), Some(Archive), Some(Archive)]), Some(Inbox));
        // 2. A rejected mail does not take a live conversation away.
        assert_eq!(f(&[Some(Banned), Some(Inbox)]), Some(Inbox));
        assert_eq!(f(&[Some(Junk), Some(Sent)]), Some(Junk));
        assert_eq!(f(&[Some(Banned), Some(Junk)]), Some(Banned));
        // 3. Only technical folders: fixed order, whatever the dates.
        assert_eq!(f(&[Some(Deleted), Some(Sent)]), Some(Sent));
        assert_eq!(f(&[Some(Quarantine), Some(Drafts)]), Some(Drafts));
        assert_eq!(f(&[Some(Deleted), Some(Quarantine)]), Some(Quarantine));
        assert_eq!(f(&[Some(Deleted)]), Some(Deleted));
        // Quarantine is technical: an archived conversation stays archived.
        assert_eq!(f(&[Some(Quarantine), Some(Archive)]), Some(Archive));
        // 4. Folders without a role only, or no mail.
        assert_eq!(f(&[None, None]), None);
        assert_eq!(f(&[]), None);
        assert_eq!(f(&[None, Some(Sent)]), Some(Sent));
    }

    #[test]
    fn move_targets_follow_spec() {
        // Sent: every command target, never Sent itself (`dossiers.md`,
        // "Cibles proposées").
        assert_eq!(
            FolderRole::Sent.move_targets(),
            [
                FolderRole::Todo,
                FolderRole::Waiting,
                FolderRole::Inbox,
                FolderRole::Archive,
                FolderRole::Newsletter,
                FolderRole::Paperwork,
                FolderRole::Junk,
                FolderRole::Banned,
                FolderRole::Deleted,
            ]
        );
        assert_eq!(FolderRole::Drafts.move_targets(), [FolderRole::Deleted]);
        assert!(
            !FolderRole::Inbox
                .move_targets()
                .contains(&FolderRole::Inbox)
        );
        assert!(
            !FolderRole::Quarantine
                .move_targets()
                .contains(&FolderRole::Todo)
        );
        assert!(
            FolderRole::Deleted
                .move_targets()
                .contains(&FolderRole::Inbox)
        );
    }

    #[test]
    fn decisions_on_move_follow_spec() {
        use FolderRole::*;
        let d = ScreeningTarget::for_move;
        // Newsletter, Paperwork, Banned decide from anywhere.
        for from in [
            Some(Inbox),
            Some(Quarantine),
            Some(Archive),
            Some(Todo),
            None,
        ] {
            assert_eq!(d(from, Newsletter), Some(ScreeningTarget::Newsletter));
            assert_eq!(d(from, Paperwork), Some(ScreeningTarget::Paperwork));
            assert_eq!(d(from, Banned), Some(ScreeningTarget::Banned));
        }
        // Inbox decides only out of Quarantine or Banned.
        assert_eq!(d(Some(Quarantine), Inbox), Some(ScreeningTarget::Inbox));
        assert_eq!(d(Some(Banned), Inbox), Some(ScreeningTarget::Inbox));
        assert_eq!(d(Some(Archive), Inbox), None);
        assert_eq!(d(Some(Deleted), Inbox), None);
        assert_eq!(d(None, Inbox), None);
        // Junk never decides, even out of Quarantine; nor the commands.
        for to in [Junk, Todo, Waiting, Archive, Deleted] {
            assert_eq!(d(Some(Quarantine), to), None);
            assert_eq!(d(Some(Inbox), to), None);
        }
    }

    #[test]
    fn filing_sources_follow_spec() {
        use FolderRole::*;
        assert_eq!(ScreeningTarget::Inbox.filing_sources(), vec![Quarantine]);
        assert_eq!(
            ScreeningTarget::Newsletter.filing_sources(),
            vec![Quarantine, Inbox, Archive, Paperwork, Banned]
        );
        for t in ScreeningTarget::ALL {
            let s = t.filing_sources();
            for never in [Todo, Waiting, Sent, Drafts, Deleted, Junk, t.folder()] {
                assert!(!s.contains(&never), "{never:?} in sources of {t:?}");
            }
            assert_eq!(ScreeningTarget::parse(t.as_str()), Some(t));
        }
    }

    #[test]
    fn auth_policy_follows_spec_table() {
        use AuthResult as V;
        // auto: pass; none and unknown only before a pass was seen.
        assert!(AuthPolicy::Auto.accepts(V::Pass, true));
        assert!(AuthPolicy::Auto.accepts(V::None, false));
        assert!(AuthPolicy::Auto.accepts(V::Unknown, false));
        assert!(!AuthPolicy::Auto.accepts(V::None, true));
        assert!(!AuthPolicy::Auto.accepts(V::Unknown, true));
        assert!(!AuthPolicy::Auto.accepts(V::Fail, false));
        // required: pass only.
        assert!(AuthPolicy::Required.accepts(V::Pass, false));
        assert!(!AuthPolicy::Required.accepts(V::None, false));
        // not_required: everything.
        assert!(AuthPolicy::NotRequired.accepts(V::Fail, true));
        // Domain rules always want pass.
        assert!(domain_rule_accepts(V::Pass));
        assert!(!domain_rule_accepts(V::Unknown));
    }

    #[test]
    fn after_send_round_trips_and_names_its_folder() {
        for a in AfterSend::ALL {
            assert_eq!(AfterSend::parse(a.as_str()), Some(a));
            // The JSON of the contract is the identifier of the database.
            assert_eq!(serde_json::to_value(a).unwrap(), a.as_str());
        }
        assert_eq!(AfterSend::parse("inbox"), None);
        assert_eq!(AfterSend::Waiting.folder(), Some(FolderRole::Waiting));
        assert_eq!(AfterSend::Stay.folder(), None);
    }

    #[test]
    fn reply_target_follows_spec() {
        use FolderRole::*;
        assert_eq!(Inbox.reply_target(None), Some(Archive));
        assert_eq!(Inbox.reply_target(Some(Waiting)), Some(Waiting));
        assert_eq!(Inbox.reply_target(Some(Todo)), Some(Todo));
        assert_eq!(Todo.reply_target(None), Some(Archive));
        assert_eq!(Waiting.reply_target(None), Some(Waiting));
        assert_eq!(Archive.reply_target(None), None);
    }
}
