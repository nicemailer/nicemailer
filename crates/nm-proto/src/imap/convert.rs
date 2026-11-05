//! From the types of `imap-codec` to ours: the only file, with
//! `session.rs` which calls it, that sees `imap-codec`.
//!
//! `imap-codec` is an alpha (`plan-imap.md`, section 2): if its types change
//! in a later version, this file is what changes, and the rest of the core
//! does not notice.
//!
//! Rust note: the decoded response *borrows* the bytes of the read buffer
//! (the `'a` lifetime of `Response<'a>`): it holds no copy, which is fast,
//! but it cannot outlive the buffer, which the next read overwrites. Each
//! response is therefore converted at once into owned values (`String`,
//! `Vec<u8>`), which can be kept.

use imap_codec::imap_types::core::{IString, NString};
use imap_codec::imap_types::extensions::namespace::Namespace as CodecNamespace;
use imap_codec::imap_types::fetch::{MessageDataItem, Section};
use imap_codec::imap_types::flag::{Flag, FlagFetch, FlagPerm};
use imap_codec::imap_types::mailbox::Mailbox;
use imap_codec::imap_types::response::{Code, Data, Response, Status, StatusBody, StatusKind};
use imap_codec::imap_types::sequence::{SeqOrUid, Sequence, SequenceSet};
use imap_codec::imap_types::status::StatusDataItem;

use super::types::*;

/// A response, owned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resp {
    /// `+`: the server waits for the rest of the command.
    Continue,
    Tagged { tag: String, status: Done },
    /// Untagged `OK`, `NO` or `BAD`.
    Status(Done),
    Bye(String),
    Data(Data_),
}

/// The status part of a response: `OK [CODE] text`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Done {
    pub kind: Kind,
    pub code: Option<RespCode>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Ok,
    No,
    Bad,
}

/// The response codes NiceMailer reads (`[UIDVALIDITY 3]`…).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RespCode {
    Alert,
    Capability(Capabilities),
    UidValidity(u32),
    UidNext(u32),
    HighestModSeq(u64),
    NoModSeq,
    PermanentFlags(Vec<String>),
    ReadOnly,
    ReadWrite,
    AppendUid(AppendUid),
    CopyUid(CopyUids),
    Modified(UidSet),
    Closed,
    /// Any other code, as text (`AUTHENTICATIONFAILED`, `OVERQUOTA`…).
    Other(String),
}

/// Untagged data, owned.
///
/// Rust note: the trailing underscore keeps the name apart from `Data` of
/// `imap-codec`, imported above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Data_ {
    Capability(Capabilities),
    List(ListEntry),
    Status(MailboxName, MailboxStatus),
    Search(Vec<u32>),
    Flags(Vec<String>),
    Exists(u32),
    Recent(u32),
    Expunge(u32),
    Fetch(Fetched),
    Enabled(Vec<String>),
    Vanished { earlier: bool, uids: UidSet },
    Namespace(Namespaces),
    /// Data NiceMailer does not use (ID, QUOTA, LSUB…).
    Other,
}

pub(crate) fn response(r: Response<'_>) -> Resp {
    match r {
        Response::CommandContinuationRequest(_) => Resp::Continue,
        Response::Status(Status::Tagged(t)) => Resp::Tagged { tag: t.tag.as_ref().to_string(), status: done(t.body) },
        Response::Status(Status::Untagged(b)) => Resp::Status(done(b)),
        Response::Status(Status::Bye(b)) => Resp::Bye(b.text.inner().to_string()),
        Response::Data(d) => Resp::Data(data(d)),
    }
}

fn done(b: StatusBody<'_>) -> Done {
    Done {
        kind: match b.kind {
            StatusKind::Ok => Kind::Ok,
            StatusKind::No => Kind::No,
            StatusKind::Bad => Kind::Bad,
        },
        code: b.code.map(code),
        text: b.text.inner().to_string(),
    }
}

pub(crate) fn code(c: Code<'_>) -> RespCode {
    match c {
        Code::Alert => RespCode::Alert,
        Code::Capability(caps) => RespCode::Capability(Capabilities::from_names(caps.as_ref().iter().map(|c| c.to_string()))),
        Code::UidValidity(n) => RespCode::UidValidity(n.get()),
        Code::UidNext(n) => RespCode::UidNext(n.get()),
        Code::HighestModSeq(n) => RespCode::HighestModSeq(n.get()),
        Code::NoModSeq => RespCode::NoModSeq,
        Code::PermanentFlags(flags) => RespCode::PermanentFlags(
            flags
                .iter()
                .map(|f| match f {
                    FlagPerm::Flag(f) => f.to_string(),
                    FlagPerm::Asterisk => "\\*".to_string(),
                })
                .collect(),
        ),
        Code::ReadOnly => RespCode::ReadOnly,
        Code::ReadWrite => RespCode::ReadWrite,
        Code::AppendUid { uid_validity, uid } => RespCode::AppendUid(AppendUid { uid_validity: uid_validity.get(), uid: uid.get() }),
        Code::CopyUid { uid_validity, source, destination } => {
            let src = uid_list(&source.0.as_ref().iter().map(uid_element).collect::<Vec<_>>());
            let dst = uid_list(&destination.0.as_ref().iter().map(uid_element).collect::<Vec<_>>());
            // RFC 4315: the two sets list the UIDs in the same order, and
            // have the same size. If not, the mapping is unknown: better
            // none than a wrong one.
            let pairs = if src.len() == dst.len() { src.into_iter().zip(dst).collect() } else { Vec::new() };
            RespCode::CopyUid(CopyUids { uid_validity: uid_validity.get(), pairs })
        }
        Code::Modified(set) => RespCode::Modified(uid_set(&set)),
        Code::Closed => RespCode::Closed,
        other => RespCode::Other(format!("{other:?}")),
    }
}

/// A `UidElement` as a range.
fn uid_element(e: &imap_codec::imap_types::extensions::uidplus::UidElement) -> (u32, u32) {
    use imap_codec::imap_types::extensions::uidplus::UidElement;
    match e {
        UidElement::Single(n) => (n.get(), n.get()),
        UidElement::Range(a, b) => (a.get(), b.get()),
    }
}

/// The UIDs of `COPYUID` in the order given (not sorted: the order is what
/// pairs a source with its destination). `3:1` counts down, as RFC 3501
/// allows ranges in either direction. At most a million: a larger set is
/// not a real answer.
fn uid_list(ranges: &[(u32, u32)]) -> Vec<u32> {
    let mut out = Vec::new();
    for &(a, b) in ranges {
        if a <= b {
            out.extend(a..=b);
        } else {
            out.extend((b..=a).rev());
        }
        if out.len() > 1_000_000 {
            return Vec::new();
        }
    }
    out
}

/// A sequence set of UIDs (`VANISHED`, `MODIFIED`). `*` cannot appear in a
/// server's answer; if it does, it is read as the largest UID.
fn uid_set(set: &SequenceSet) -> UidSet {
    let v = |s: &SeqOrUid| match s {
        SeqOrUid::Value(n) => n.get(),
        SeqOrUid::Asterisk => u32::MAX,
    };
    UidSet::from_ranges(set.0.as_ref().iter().map(|s| match s {
        Sequence::Single(a) => (v(a), v(a)),
        Sequence::Range(a, b) => (v(a), v(b)),
    }))
}

fn data(d: Data<'_>) -> Data_ {
    match d {
        Data::Capability(caps) => Data_::Capability(Capabilities::from_names(caps.as_ref().iter().map(|c| c.to_string()))),
        Data::List { items, delimiter, mailbox } => Data_::List(ListEntry {
            name: mailbox_name(&mailbox),
            delimiter: delimiter.map(|d| d.inner()),
            attributes: items.iter().map(|a| a.to_string()).collect(),
        }),
        Data::Status { mailbox, items } => {
            let mut s = MailboxStatus::default();
            for item in items.iter() {
                match item {
                    StatusDataItem::Messages(n) => s.messages = Some(*n),
                    StatusDataItem::UidNext(n) => s.uid_next = Some(n.get()),
                    StatusDataItem::UidValidity(n) => s.uid_validity = Some(n.get()),
                    StatusDataItem::Unseen(n) => s.unseen = Some(*n),
                    StatusDataItem::HighestModSeq(n) => s.highest_modseq = Some(*n),
                    _ => {}
                }
            }
            Data_::Status(mailbox_name(&mailbox), s)
        }
        Data::Search(ids, _) => Data_::Search(ids.iter().map(|n| n.get()).collect()),
        Data::Flags(flags) => Data_::Flags(flags.iter().map(Flag::to_string).collect()),
        Data::Exists(n) => Data_::Exists(n),
        Data::Recent(n) => Data_::Recent(n),
        Data::Expunge(n) => Data_::Expunge(n.get()),
        Data::Fetch { seq, items } => Data_::Fetch(fetched(seq.get(), items.into_inner())),
        Data::Enabled { capabilities } => Data_::Enabled(capabilities.iter().map(|c| c.to_string().to_ascii_uppercase()).collect()),
        Data::Vanished { earlier, known_uids } => Data_::Vanished { earlier, uids: uid_set(&known_uids) },
        Data::Namespace { personal, other, shared } => Data_::Namespace(Namespaces {
            personal: personal.iter().map(namespace).collect(),
            other: other.iter().map(namespace).collect(),
            shared: shared.iter().map(namespace).collect(),
        }),
        _ => Data_::Other,
    }
}

fn namespace(n: &CodecNamespace<'_>) -> Namespace {
    Namespace { prefix: String::from_utf8_lossy(n.prefix.as_ref()).into_owned(), delimiter: n.delimiter.map(|d| d.inner()) }
}

pub(crate) fn mailbox_name(m: &Mailbox<'_>) -> MailboxName {
    match m {
        Mailbox::Inbox => MailboxName::inbox(),
        Mailbox::Other(o) => MailboxName::from_wire(o.as_ref().to_vec()),
    }
}

fn fetched(seq: u32, items: Vec<MessageDataItem<'_>>) -> Fetched {
    let mut f = Fetched { seq, ..Fetched::default() };
    for item in items {
        match item {
            MessageDataItem::Uid(n) => f.uid = Some(n.get()),
            MessageDataItem::Flags(flags) => {
                f.flags = Some(
                    flags
                        .iter()
                        .filter_map(|fl| match fl {
                            FlagFetch::Flag(fl) => Some(fl.to_string()),
                            // \Recent is a property of the session, not of
                            // the message, and cannot be stored.
                            FlagFetch::Recent => None,
                        })
                        .collect(),
                )
            }
            MessageDataItem::InternalDate(d) => f.internal_date = Some(d.as_ref().timestamp()),
            MessageDataItem::Rfc822Size(n) => f.size = Some(n),
            MessageDataItem::ModSeq(n) => f.modseq = Some(n.get()),
            MessageDataItem::BodyExt { section, data, .. } => {
                let bytes = nstring(data);
                match section {
                    // BODY[HEADER]
                    Some(Section::Header(None)) => f.header = Some(bytes),
                    // BODY[] or BODY[]<origin>
                    None => f.body = Some(bytes),
                    _ => {}
                }
            }
            MessageDataItem::Rfc822Header(data) => f.header = Some(nstring(data)),
            MessageDataItem::Rfc822(data) => f.body = Some(nstring(data)),
            _ => {}
        }
    }
    f
}

/// The bytes of an `nstring`; `NIL` gives no bytes.
fn nstring(n: NString<'_>) -> Vec<u8> {
    match n.0 {
        Some(IString::Literal(l)) => l.into_inner().into_owned(),
        Some(IString::Quoted(q)) => q.into_inner().into_owned().into_bytes(),
        None => Vec::new(),
    }
}

