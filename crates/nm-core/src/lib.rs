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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
    /// - Sent and Drafts offer nothing: a sent message stays in Sent, and a
    ///   draft is either sent or deleted.
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
            Sent | Drafts => vec![],
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
    fn move_targets_follow_spec() {
        assert!(FolderRole::Sent.move_targets().is_empty());
        assert!(!FolderRole::Inbox.move_targets().contains(&FolderRole::Inbox));
        assert!(!FolderRole::Quarantine.move_targets().contains(&FolderRole::Todo));
        assert!(FolderRole::Deleted.move_targets().contains(&FolderRole::Inbox));
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
