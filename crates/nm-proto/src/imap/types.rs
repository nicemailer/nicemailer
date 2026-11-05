//! What the IMAP client gives the rest of the core: plain owned types,
//! independent of `imap-codec`, whose types stay inside this module.

use std::collections::BTreeSet;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// How the connection is protected (`configuration.md`, `imap.security`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Security {
    /// TLS from the first byte (port 993).
    Tls,
    /// A plain connection upgraded by `STARTTLS` (port 143). Refused if the
    /// server does not upgrade: never a fallback to clear text.
    Starttls,
    /// Clear text, only to the loopback address (test servers).
    None,
}

/// The server to connect to.
#[derive(Debug, Clone)]
pub struct ServerSpec {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// An extra certificate authority (PEM), trusted on top of the
    /// system's: for a self-hosted server and the test servers.
    pub ca_file: Option<PathBuf>,
}

/// Time limits. Each read and each write has its own limit: a large
/// download that keeps flowing never times out, a server that stops
/// answering does.
#[derive(Debug, Clone, Copy)]
pub struct Timeouts {
    /// Opening the connection, TLS included.
    pub connect: Duration,
    /// Silence of the server while a response is expected, and time to
    /// write a piece of a command.
    pub io: Duration,
}

impl Default for Timeouts {
    fn default() -> Timeouts {
        Timeouts { connect: Duration::from_secs(30), io: Duration::from_secs(90) }
    }
}

/// The capabilities a server announces, in upper case (`IMAP4REV1`,
/// `AUTH=PLAIN`, `QRESYNC`…).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Capabilities(pub BTreeSet<String>);

impl Capabilities {
    pub fn from_names<I: IntoIterator<Item = S>, S: AsRef<str>>(names: I) -> Capabilities {
        Capabilities(names.into_iter().map(|n| n.as_ref().to_ascii_uppercase()).collect())
    }

    /// Whether `name` is announced; case does not matter.
    pub fn has(&self, name: &str) -> bool {
        self.0.contains(&name.to_ascii_uppercase())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
}

/// The name of a mailbox, as the server sent it and as shown.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MailboxName {
    /// The bytes on the wire, in modified UTF-7: what is sent back to the
    /// server, never re-encoded.
    pub raw: Vec<u8>,
    /// The name decoded; the raw bytes as they are (lossy) when they are not
    /// valid modified UTF-7.
    pub name: String,
}

impl MailboxName {
    /// A name received from the server.
    pub fn from_wire(raw: Vec<u8>) -> MailboxName {
        let name = super::utf7::decode(&raw).unwrap_or_else(|| String::from_utf8_lossy(&raw).into_owned());
        MailboxName { raw, name }
    }

    /// A name chosen by NiceMailer (a folder to create), encoded.
    pub fn new(name: &str) -> MailboxName {
        MailboxName { raw: super::utf7::encode(name).into_bytes(), name: name.to_string() }
    }

    pub fn inbox() -> MailboxName {
        MailboxName::new("INBOX")
    }

    /// INBOX is the one name whose case does not matter (RFC 3501, 5.1).
    pub fn is_inbox(&self) -> bool {
        self.raw.eq_ignore_ascii_case(b"INBOX")
    }
}

impl fmt::Display for MailboxName {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.name)
    }
}

/// The special uses of RFC 6154.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SpecialUse {
    All,
    Archive,
    Drafts,
    Flagged,
    Junk,
    Sent,
    Trash,
}

impl SpecialUse {
    /// From a `LIST` attribute (`\Sent`), case-insensitive.
    pub fn from_attribute(attr: &str) -> Option<SpecialUse> {
        Some(match attr.to_ascii_lowercase().as_str() {
            "\\all" => SpecialUse::All,
            "\\archive" => SpecialUse::Archive,
            "\\drafts" => SpecialUse::Drafts,
            "\\flagged" => SpecialUse::Flagged,
            "\\junk" => SpecialUse::Junk,
            "\\sent" => SpecialUse::Sent,
            "\\trash" => SpecialUse::Trash,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            SpecialUse::All => "all",
            SpecialUse::Archive => "archive",
            SpecialUse::Drafts => "drafts",
            SpecialUse::Flagged => "flagged",
            SpecialUse::Junk => "junk",
            SpecialUse::Sent => "sent",
            SpecialUse::Trash => "trash",
        }
    }
}

/// One mailbox of a `LIST` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListEntry {
    pub name: MailboxName,
    /// Separator of the hierarchy (`/`, `.`), `None` for a flat namespace.
    pub delimiter: Option<char>,
    /// Attributes as sent (`\HasNoChildren`, `\Sent`…).
    pub attributes: Vec<String>,
}

impl ListEntry {
    /// `\Noselect` or `\NonExistent`: a node of the hierarchy that holds no
    /// mail and cannot be selected.
    pub fn selectable(&self) -> bool {
        !self.attributes.iter().any(|a| a.eq_ignore_ascii_case("\\Noselect") || a.eq_ignore_ascii_case("\\NonExistent"))
    }

    /// The special use the server declares, if any. A server should give at
    /// most one; if it gives several, the first one counts.
    pub fn special_use(&self) -> Option<SpecialUse> {
        self.attributes.iter().find_map(|a| SpecialUse::from_attribute(a))
    }
}

/// A namespace of the `NAMESPACE` response (RFC 2342).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Namespace {
    /// Prefix of the names (`""`, `INBOX.`).
    pub prefix: String,
    pub delimiter: Option<char>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Namespaces {
    pub personal: Vec<Namespace>,
    pub other: Vec<Namespace>,
    pub shared: Vec<Namespace>,
}

/// The `STATUS` of a mailbox: what is asked and announced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailboxStatus {
    pub messages: Option<u32>,
    pub uid_next: Option<u32>,
    pub uid_validity: Option<u32>,
    pub unseen: Option<u32>,
    /// With CONDSTORE only.
    pub highest_modseq: Option<u64>,
}

/// A set of UIDs, as ranges sorted and merged: `1:5,7,9:12`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UidSet(Vec<(u32, u32)>);

impl UidSet {
    /// From any UIDs, in any order, with repeats; 0 is not a UID and is
    /// ignored.
    pub fn from_uids<I: IntoIterator<Item = u32>>(uids: I) -> UidSet {
        let mut v: Vec<u32> = uids.into_iter().filter(|&u| u != 0).collect();
        v.sort_unstable();
        v.dedup();
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for u in v {
            match ranges.last_mut() {
                Some((_, end)) if *end + 1 == u => *end = u,
                _ => ranges.push((u, u)),
            }
        }
        UidSet(ranges)
    }

    /// From ranges, in any order, overlapping or not.
    pub fn from_ranges<I: IntoIterator<Item = (u32, u32)>>(ranges: I) -> UidSet {
        let mut v: Vec<(u32, u32)> = ranges
            .into_iter()
            .map(|(a, b)| (a.min(b), a.max(b)))
            .filter(|&(_, b)| b != 0)
            .map(|(a, b)| (a.max(1), b))
            .collect();
        v.sort_unstable();
        let mut out: Vec<(u32, u32)> = Vec::with_capacity(v.len());
        for (a, b) in v {
            match out.last_mut() {
                Some((_, end)) if a <= end.saturating_add(1) => *end = (*end).max(b),
                _ => out.push((a, b)),
            }
        }
        UidSet(out)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Number of UIDs.
    pub fn len(&self) -> u64 {
        self.0.iter().map(|(a, b)| u64::from(b - a) + 1).sum()
    }

    pub fn ranges(&self) -> &[(u32, u32)] {
        &self.0
    }

    pub fn contains(&self, uid: u32) -> bool {
        // Rust note: `binary_search_by` finds a range by comparing it with
        // the UID: `Less` if the range is before it, `Greater` if after.
        self.0
            .binary_search_by(|&(a, b)| {
                if b < uid {
                    std::cmp::Ordering::Less
                } else if a > uid {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    /// Every UID, in order. Sets of ranges can be large (`1:4000000000`):
    /// iterate, do not collect blindly.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.0.iter().flat_map(|&(a, b)| a..=b)
    }
}

impl UidSet {
    /// The set cut into pieces whose text is at most `max_len` bytes, in
    /// order: servers limit the length of a command line (8 KiB is safe
    /// everywhere), and a list of scattered UIDs can be far longer.
    pub fn chunks(&self, max_len: usize) -> Vec<UidSet> {
        let mut out = Vec::new();
        let mut current: Vec<(u32, u32)> = Vec::new();
        let mut len = 0;
        for &(a, b) in &self.0 {
            let piece = if a == b { a.to_string().len() } else { a.to_string().len() + 1 + b.to_string().len() };
            if !current.is_empty() && len + 1 + piece > max_len {
                out.push(UidSet(std::mem::take(&mut current)));
                len = 0;
            }
            len += piece + usize::from(!current.is_empty());
            current.push((a, b));
        }
        if !current.is_empty() {
            out.push(UidSet(current));
        }
        out
    }
}

/// The UIDs a command applies to: a set, or every UID from one on
/// (`n:*`, the new messages).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Uids {
    Set(UidSet),
    From(u32),
}

impl fmt::Display for UidSet {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (i, (a, b)) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            if a == b {
                write!(f, "{a}")?;
            } else {
                write!(f, "{a}:{b}")?;
            }
        }
        Ok(())
    }
}

/// What a `FETCH` asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FetchItems {
    pub flags: bool,
    pub internal_date: bool,
    pub size: bool,
    /// With CONDSTORE only.
    pub modseq: bool,
    /// The whole header (`BODY.PEEK[HEADER]`).
    pub header: bool,
    /// The message (`BODY.PEEK[]`), whole or a piece. `PEEK`: fetching a
    /// body never marks the message read.
    pub body: Option<BodyRange>,
    /// `CHANGEDSINCE` (CONDSTORE): only the messages whose modseq is
    /// higher.
    pub changed_since: Option<u64>,
    /// With `changed_since` and QRESYNC: also list the UIDs expunged since
    /// then (`VANISHED (EARLIER)`, [`Unsolicited::Vanished`]).
    pub vanished: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyRange {
    Whole,
    /// `len` bytes from `offset` (`BODY.PEEK[]<offset.len>`).
    Partial { offset: u32, len: u32 },
}

/// One message of a `FETCH` response. A field is `None` when it was not
/// asked for, or the server did not send it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Fetched {
    /// Sequence number, valid only until the next expunge.
    pub seq: u32,
    pub uid: Option<u32>,
    /// Flags and keywords as sent (`\Seen`, `$Junk`…), `\Recent` excluded.
    pub flags: Option<Vec<String>>,
    /// Date of arrival on the server, Unix seconds.
    pub internal_date: Option<i64>,
    pub size: Option<u32>,
    pub modseq: Option<u64>,
    pub header: Option<Vec<u8>>,
    pub body: Option<Vec<u8>>,
}

/// What the server reported without being asked: new mail, expunged mail,
/// flags changed by another client. Kept in order and never dropped
/// ([`super::Session::take_unsolicited`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unsolicited {
    /// The mailbox now holds this many messages.
    Exists(u32),
    /// The message with this sequence number was expunged; the following
    /// ones moved down by one.
    Expunge(u32),
    /// These UIDs were expunged (QRESYNC). `earlier`: an answer about the
    /// past (`SELECT … QRESYNC`, `FETCH … VANISHED`), not a new expunge.
    Vanished { earlier: bool, uids: UidSet },
    /// Flags (or other data) of a message changed.
    Fetch(Fetched),
    /// The flags the mailbox knows changed.
    Flags(Vec<String>),
    /// A message the server says the user must see (`[ALERT]`).
    Alert(String),
}

/// Result of `SELECT` or `EXAMINE`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selected {
    pub exists: u32,
    pub uid_validity: Option<u32>,
    pub uid_next: Option<u32>,
    /// With CONDSTORE; `None` also when the server said `NOMODSEQ`.
    pub highest_modseq: Option<u64>,
    /// The mailbox does not keep modseqs (`[NOMODSEQ]`).
    pub no_modseq: bool,
    pub flags: Vec<String>,
    /// Flags that can be stored for good; `\*` means any keyword.
    pub permanent_flags: Vec<String>,
    pub read_only: bool,
    /// With QRESYNC: UIDs expunged since the known modseq.
    pub vanished: UidSet,
    /// With QRESYNC: messages changed since the known modseq.
    pub changed: Vec<Fetched>,
}

/// The QRESYNC parameters of a `SELECT` (RFC 7162, section 3.2.5): what the
/// client knew of the mailbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QresyncParams {
    pub uid_validity: u32,
    pub modseq: u64,
    /// The UIDs the client has, to limit the `VANISHED` list.
    pub known_uids: Option<UidSet>,
}

/// UIDs given by `COPYUID` (RFC 4315): where each message went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopyUids {
    pub uid_validity: u32,
    /// (source UID, destination UID), in order.
    pub pairs: Vec<(u32, u32)>,
}

/// Result of `APPEND`: the UID of the new message, with `UIDPLUS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppendUid {
    pub uid_validity: u32,
    pub uid: u32,
}

/// How a `STORE` changes the flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOp {
    Add,
    Remove,
    Replace,
}

/// Result of `STORE`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StoreResult {
    /// The new flags of each message (absent with `.SILENT`).
    pub fetched: Vec<Fetched>,
    /// With `UNCHANGEDSINCE`: the UIDs not changed because they changed on
    /// the server in between (`[MODIFIED …]`).
    pub modified: UidSet,
}

/// Result of a move.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MoveResult {
    /// Where the messages went, if the server says (`UIDPLUS`).
    pub copy_uids: Option<CopyUids>,
    /// False when the server has neither `MOVE` nor `UIDPLUS`: the messages
    /// were copied and flagged `\Deleted`, but not expunged, since a plain
    /// `EXPUNGE` would also expunge messages flagged by someone else.
    pub expunged: bool,
}

/// Why `IDLE` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleEnd {
    /// The server reported something: see
    /// [`super::Session::take_unsolicited`].
    Changes,
    /// The time asked for passed; IDLE must be started again to keep
    /// watching (RFC 2177 asks to renew it within 29 minutes).
    Timeout,
    /// The caller asked to stop.
    Stopped,
}

/// A criterion of `UID SEARCH`. Only those NiceMailer needs; each value is
/// quoted by the encoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    All,
    Uid(UidSet),
    /// Arrived on or after this day (internal date).
    Since { year: i32, month: u32, day: u32 },
    /// A header containing a value (`HEADER Message-ID <x@y>`).
    Header(String, String),
    Unseen,
    Deleted,
    /// Both criteria.
    And(Box<SearchKey>, Box<SearchKey>),
}
