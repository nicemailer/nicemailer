//! One IMAP connection: commands sent one at a time, responses read and
//! sorted between the command's result and what the server reports on its
//! own ([`Notice`]).
//!
//! # Reading
//!
//! The bytes from the server go into the `Fragmentizer` of `imap-codec`,
//! which cuts them into complete responses, literals included, without
//! decoding them; each complete response is then decoded and converted at
//! once (`convert.rs`). The fragmentizer refuses a response larger than
//! [`MAX_RESPONSE`]: memory stays bounded whatever the server sends, and
//! the framing stays right, so the session goes on after the refusal.
//! Message bodies larger than [`WHOLE_BODY_MAX`] are fetched in pieces
//! ([`Session::fetch_body`]), which keeps every response under the limit.
//!
//! # Time limits
//!
//! Each read and each write has the limit of [`Timeouts::io`]: a large
//! download that keeps flowing never times out, a server that stops
//! answering does. IDLE has no limit on the server's silence (that is its
//! purpose), only the duration the caller gives.
//!
//! # Failures
//!
//! A fatal error ([`ImapError::is_fatal`]) closes the session: the state
//! of the conversation is unknown, and going on could read the bytes of a
//! literal as a response. Every later call fails with
//! [`ImapError::Closed`].
//!
//! A response that cannot be decoded is not fatal: the fragmentizer found
//! its end, so the next one is read correctly. The command it belongs to
//! ends with [`ImapError::Malformed`], since its result may lack something.

use std::collections::{BTreeSet, VecDeque};
use std::future::Future;
use std::time::Duration;

use imap_codec::fragmentizer::{DecodeMessageError, Fragmentizer};
use imap_codec::imap_types::response::GreetingKind;
use imap_codec::{GreetingCodec, ResponseCodec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::convert::{self, Data_, Done, Kind, Resp, RespCode};
use super::encode::{self, Command, Part};
use super::error::{ImapError, Result};
use super::transport::{self, Conn, Stream};
use super::types::*;

/// Largest response kept in memory: 16 MiB. A body is fetched whole up to
/// [`WHOLE_BODY_MAX`], so a normal response is far below; the margin covers
/// a long `SEARCH` answer or a server that sends more than asked.
pub const MAX_RESPONSE: u32 = 16 << 20;
/// Bodies up to 4 MiB are fetched in one response, larger ones in pieces of
/// [`BODY_CHUNK`] (`plan-imap.md`, section 2, "Gros messages").
pub const WHOLE_BODY_MAX: u32 = 4 << 20;
pub const BODY_CHUNK: u32 = 1 << 20;
/// IDLE is renewed after at most 25 minutes: RFC 2177 lets a server end it
/// after 30, and asks clients to renew it within 29.
pub const IDLE_MAX: Duration = Duration::from_secs(25 * 60);
/// Longest set of UIDs in one command; a longer one is split.
const MAX_SET_LEN: usize = 4000;
/// Size of the read buffer.
const READ_BUF: usize = 64 * 1024;

/// What the server reported on its own, with the mailbox it concerns (the
/// one selected when it arrived).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub mailbox: Option<MailboxName>,
    pub event: Unsolicited,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    NotAuthenticated,
    Authenticated,
    Selected { mailbox: MailboxName, read_only: bool },
}

/// The untagged responses of a command.
#[derive(Debug)]
enum Untagged {
    Data(Data_),
    Status(Done),
}

/// What a command received: its untagged responses and its end.
struct Reply {
    untagged: Vec<Untagged>,
    done: Done,
}

/// A connection to an IMAP server.
///
/// Rust note: the methods take `&mut self`: one command at a time, by
/// construction. The borrow checker refuses two calls in flight on the same
/// session, so responses can never be mixed up between commands.
pub struct Session {
    /// `None` once closed, after LOGOUT or a fatal error.
    conn: Option<Conn>,
    frag: Fragmentizer,
    buf: Vec<u8>,
    next_tag: u32,
    timeouts: Timeouts,
    caps: Capabilities,
    caps_before_login: Capabilities,
    state: State,
    /// Extensions turned on by `ENABLE` (or implied by a `SELECT` with
    /// `CONDSTORE`), in upper case.
    enabled: BTreeSet<String>,
    notices: VecDeque<Notice>,
    /// Text of the `BYE` received, if any: the reason given when the
    /// connection then closes.
    bye: Option<String>,
    /// A response of the current command that could not be read.
    malformed: Option<String>,
    /// Writing commands refused before they leave (`forbid_writes`).
    read_only: bool,
}

impl Session {
    /// Connects to the server and reads its greeting, and its capabilities.
    /// With [`Security::Starttls`], upgrades to TLS before anything else;
    /// the capabilities announced before are then forgotten (RFC 3501,
    /// 6.2.1), since anyone on the network could have changed them.
    pub async fn connect(spec: &ServerSpec, timeouts: Timeouts) -> Result<Session> {
        let conn = transport::open(spec, &timeouts).await?;
        let mut s = Session::new(conn, timeouts);
        s.greeting().await?;
        if spec.security == Security::Starttls {
            s.starttls(spec).await?;
        }
        if s.caps.0.is_empty() {
            s.capability().await?;
        }
        s.caps_before_login = s.caps.clone();
        Ok(s)
    }

    /// A session over any stream, already open: for tests (a scripted
    /// server) and tunnels. Reads the greeting.
    pub async fn over(stream: impl Stream + 'static, timeouts: Timeouts) -> Result<Session> {
        let mut s = Session::new(Conn::Other(Box::new(stream)), timeouts);
        s.greeting().await?;
        if s.caps.0.is_empty() {
            s.capability().await?;
        }
        s.caps_before_login = s.caps.clone();
        Ok(s)
    }

    fn new(conn: Conn, timeouts: Timeouts) -> Session {
        Session {
            conn: Some(conn),
            frag: Fragmentizer::new(MAX_RESPONSE),
            buf: vec![0; READ_BUF],
            next_tag: 0,
            timeouts,
            caps: Capabilities::default(),
            caps_before_login: Capabilities::default(),
            state: State::NotAuthenticated,
            enabled: BTreeSet::new(),
            notices: VecDeque::new(),
            bye: None,
            malformed: None,
            read_only: false,
        }
    }

    // ---------------------------------------------------------------------
    // State
    // ---------------------------------------------------------------------

    /// Capabilities as of now (after login, those of the authenticated
    /// state).
    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    /// Capabilities announced before login: they only serve to choose how
    /// to log in.
    pub fn capabilities_before_login(&self) -> &Capabilities {
        &self.caps_before_login
    }

    /// Extensions turned on, in upper case (`CONDSTORE`, `QRESYNC`).
    pub fn enabled(&self) -> impl Iterator<Item = &str> {
        self.enabled.iter().map(String::as_str)
    }

    /// Whether modseqs are available: CONDSTORE or QRESYNC on.
    pub fn condstore(&self) -> bool {
        self.enabled.contains("CONDSTORE") || self.enabled.contains("QRESYNC")
    }

    pub fn qresync(&self) -> bool {
        self.enabled.contains("QRESYNC")
    }

    pub fn is_open(&self) -> bool {
        self.conn.is_some()
    }

    pub fn is_tls(&self) -> bool {
        self.conn.as_ref().is_some_and(Conn::is_tls)
    }

    /// The mailbox selected, and whether read-only.
    pub fn selected(&self) -> Option<(&MailboxName, bool)> {
        match &self.state {
            State::Selected { mailbox, read_only } => Some((mailbox, *read_only)),
            _ => None,
        }
    }

    /// What the server reported on its own since the last call, in order.
    /// Nothing is ever dropped: a caller that does not call this keeps it
    /// all in memory.
    pub fn take_unsolicited(&mut self) -> Vec<Notice> {
        self.notices.drain(..).collect()
    }

    // ---------------------------------------------------------------------
    // Reading and writing
    // ---------------------------------------------------------------------

    /// Closes the session after a fatal error, and returns the error.
    fn fail(&mut self, e: ImapError) -> ImapError {
        if e.is_fatal() {
            self.conn = None;
        }
        e
    }

    /// Reads bytes until the fragmentizer holds a complete response.
    /// `limit`: the longest silence allowed, `None` for none (IDLE).
    ///
    /// Cancel-safe: the only await is one read, whose bytes go straight into
    /// the fragmentizer, which lives in `self`. If the future is dropped
    /// (IDLE stopped), nothing read is lost.
    async fn read_message(&mut self, limit: Option<Duration>) -> Result<()> {
        loop {
            if self.frag.progress().is_some() {
                if self.frag.is_message_complete() {
                    return Ok(());
                }
                continue;
            }
            let Some(conn) = self.conn.as_mut() else { return Err(ImapError::Closed) };
            let read = conn.read(&mut self.buf);
            let n = match limit {
                Some(d) => tokio::time::timeout(d, read).await.map_err(|_| ImapError::Timeout("waiting for the server"))?,
                None => read.await,
            }
            .map_err(|e| ImapError::Io(e.to_string()))?;
            if n == 0 {
                return Err(match self.bye.take() {
                    Some(text) => ImapError::Bye(text),
                    None => ImapError::Closed,
                });
            }
            self.frag.enqueue_bytes(&self.buf[..n]);
        }
    }

    /// The next response, decoded.
    async fn read_resp(&mut self, limit: Option<Duration>) -> Result<std::result::Result<Resp, (Option<String>, String)>> {
        self.read_message(limit).await?;
        Ok(match self.frag.decode_message(&ResponseCodec::default()) {
            Ok(r) => Ok(convert::response(r)),
            Err(DecodeMessageError::MessageTooLong { .. }) => {
                Err((self.frag.decode_tag().map(|t| t.as_ref().to_string()), format!("response larger than {} MiB", MAX_RESPONSE >> 20)))
            }
            Err(_) => {
                let bytes = self.frag.message_bytes();
                // RFC 3501 forbids NUL in a literal, and imap-codec refuses
                // it; a stored message can still hold one, and a server
                // that sends it as is would make that message unreadable
                // for good, the synchronization stuck on it. Dovecot sends
                // 0x80 in its place: do the same, on a copy, before giving
                // up. A NUL cannot stand anywhere else in a response.
                if bytes.contains(&0) {
                    let fixed: Vec<u8> = bytes.iter().map(|&b| if b == 0 { 0x80 } else { b }).collect();
                    if let Ok((rest, r)) = imap_codec::decode::Decoder::decode(&ResponseCodec::default(), &fixed)
                        && rest.is_empty()
                    {
                        return Ok(Ok(convert::response(r)));
                    }
                }
                // A UIDVALIDITY of 0, which RFC 3501 forbids (section
                // 2.3.1.1, a non-zero number) and imap-codec refuses:
                // Dovecot gives it for a folder it is still creating, when
                // several sessions of a new user open at once. It means "not
                // known yet": the response is read without it, and the pass
                // opens the folder to learn the real one (or skips it until
                // the next pass if `EXAMINE` gives none).
                if let Some(fixed) = without_zero_uidvalidity(bytes)
                    && let Ok((rest, r)) = imap_codec::decode::Decoder::decode(&ResponseCodec::default(), &fixed)
                    && rest.is_empty()
                {
                    return Ok(Ok(convert::response(r)));
                }
                let sample = String::from_utf8_lossy(&bytes[..bytes.len().min(160)]).into_owned();
                Err((self.frag.decode_tag().map(|t| t.as_ref().to_string()), format!("cannot read {:?}", sample.trim_end())))
            }
        })
    }

    async fn greeting(&mut self) -> Result<()> {
        let r = self.greeting_inner().await;
        r.map_err(|e| self.fail(e))
    }

    async fn greeting_inner(&mut self) -> Result<()> {
        self.read_message(Some(self.timeouts.io)).await?;
        let g = self
            .frag
            .decode_message(&GreetingCodec::default())
            .map_err(|_| ImapError::Protocol("the server's greeting cannot be read: is this an IMAP server?".into()))?;
        let kind = g.kind;
        let text = g.text.inner().to_string();
        if let Some(RespCode::Capability(c)) = g.code.map(convert::code) {
            self.caps = c;
        }
        match kind {
            GreetingKind::Ok => {}
            GreetingKind::PreAuth => self.state = State::Authenticated,
            GreetingKind::Bye => return Err(ImapError::Bye(text)),
        }
        Ok(())
    }

    /// Writes bytes, in pieces each under the time limit: a large APPEND on
    /// a slow line goes through as long as it moves.
    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let io = self.timeouts.io;
        let Some(conn) = self.conn.as_mut() else { return Err(ImapError::Closed) };
        for piece in bytes.chunks(READ_BUF) {
            tokio::time::timeout(io, conn.write_all(piece))
                .await
                .map_err(|_| ImapError::Timeout("sending"))?
                .map_err(|e| ImapError::Io(e.to_string()))?;
        }
        tokio::time::timeout(io, conn.flush())
            .await
            .map_err(|_| ImapError::Timeout("sending"))?
            .map_err(|e| ImapError::Io(e.to_string()))
    }

    fn tag(&mut self) -> String {
        self.next_tag += 1;
        format!("N{}", self.next_tag)
    }

    /// Runs a command: sends it, waits for its end, and returns what it
    /// received. A `NO` or a `BAD` is an `Ok` here: [`Session::check`]
    /// turns them into errors, after the untagged responses were sorted.
    async fn run(&mut self, cmd: Command) -> Result<Reply> {
        let r = self.run_inner(cmd).await;
        r.map_err(|e| self.fail(e))
    }

    async fn run_inner(&mut self, cmd: Command) -> Result<Reply> {
        if self.conn.is_none() {
            return Err(ImapError::Closed);
        }
        self.malformed = None;
        let tag = self.tag();
        let mut untagged = Vec::new();
        let mut line: Vec<u8> = Vec::with_capacity(256);
        line.extend_from_slice(tag.as_bytes());
        line.push(b' ');
        for part in cmd.parts {
            match part {
                Part::Text(t) => line.extend_from_slice(&t),
                Part::Literal(data) => {
                    // LITERAL+ (RFC 7888): the literal goes at once, `{n+}`;
                    // LITERAL- allows it up to 4096 bytes. Otherwise the
                    // server must say `+` first.
                    let non_sync = self.caps.has("LITERAL+") || (self.caps.has("LITERAL-") && data.len() <= 4096);
                    line.extend_from_slice(format!("{{{}{}}}\r\n", data.len(), if non_sync { "+" } else { "" }).as_bytes());
                    self.write(&line).await?;
                    line.clear();
                    if !non_sync && let Some(done) = self.wait_continue(&tag, &mut untagged).await? {
                        return Ok(Reply { untagged, done });
                    }
                    self.write(&data).await?;
                }
                Part::Wait => {
                    line.extend_from_slice(b"\r\n");
                    self.write(&line).await?;
                    line.clear();
                    if let Some(done) = self.wait_continue(&tag, &mut untagged).await? {
                        return Ok(Reply { untagged, done });
                    }
                }
            }
        }
        line.extend_from_slice(b"\r\n");
        self.write(&line).await?;
        loop {
            match self.next_for(&tag, &mut untagged).await? {
                Step::Done(done) => return Ok(Reply { untagged, done }),
                Step::Continue => return Err(ImapError::Protocol(format!("unexpected '+' during {}", cmd.name))),
                Step::More => {}
            }
        }
    }

    /// Waits for the `+` that lets the rest of the command go. Returns the
    /// end of the command if the server ends it instead (a `NO` to a
    /// literal it refuses).
    async fn wait_continue(&mut self, tag: &str, untagged: &mut Vec<Untagged>) -> Result<Option<Done>> {
        loop {
            match self.next_for(tag, untagged).await? {
                Step::Continue => return Ok(None),
                Step::Done(done) => return Ok(Some(done)),
                Step::More => {}
            }
        }
    }

    /// Reads one response of the command `tag`.
    async fn next_for(&mut self, tag: &str, untagged: &mut Vec<Untagged>) -> Result<Step> {
        match self.read_resp(Some(self.timeouts.io)).await? {
            Ok(Resp::Continue) => Ok(Step::Continue),
            Ok(Resp::Tagged { tag: t, status }) if t == tag => Ok(Step::Done(status)),
            Ok(Resp::Tagged { tag: t, .. }) => Err(ImapError::Protocol(format!("response to an unknown command {t}"))),
            Ok(Resp::Status(s)) => {
                untagged.push(Untagged::Status(s));
                Ok(Step::More)
            }
            Ok(Resp::Bye(text)) => {
                // The connection closes next, unless this is LOGOUT, whose
                // OK follows.
                self.bye = Some(text);
                Ok(Step::More)
            }
            Ok(Resp::Data(d)) => {
                untagged.push(Untagged::Data(d));
                Ok(Step::More)
            }
            Err((t, detail)) => {
                self.malformed.get_or_insert(detail.clone());
                if t.as_deref() == Some(tag) {
                    // The end of the command, unreadable: it is over, its
                    // outcome unknown.
                    return Ok(Step::Done(Done { kind: Kind::Bad, code: None, text: detail }));
                }
                Ok(Step::More)
            }
        }
    }

    /// Turns the end of a command into a result: the untagged responses not
    /// taken by the command go to the notices, then `NO`, `BAD` and an
    /// unreadable response become errors.
    fn check(&mut self, name: &'static str, reply: Reply) -> Result<(Done, Vec<Untagged>)> {
        if let Some(detail) = self.malformed.take() {
            self.absorb(reply.untagged);
            return Err(ImapError::Malformed { command: name, detail });
        }
        match reply.done.kind {
            Kind::Ok => Ok((reply.done, reply.untagged)),
            Kind::No => {
                self.absorb(reply.untagged);
                Err(ImapError::No { command: name, text: reply.done.text, code: reply.done.code.map(code_name) })
            }
            Kind::Bad => {
                self.absorb(reply.untagged);
                Err(ImapError::Bad { command: name, text: reply.done.text })
            }
        }
    }

    /// Runs a command and checks its end.
    async fn exec(&mut self, cmd: Command) -> Result<(Done, Vec<Untagged>)> {
        let name = cmd.name;
        let reply = self.run(cmd).await?;
        self.check(name, reply)
    }

    /// Keeps what the server reported on its own: the untagged responses a
    /// command did not take.
    fn absorb(&mut self, untagged: Vec<Untagged>) {
        for u in untagged {
            self.absorb_one(u);
        }
    }

    /// Keeps one untagged response if it reports something; returns whether
    /// it did (for IDLE).
    fn absorb_one(&mut self, u: Untagged) -> bool {
        let event = match u {
            Untagged::Data(Data_::Exists(n)) => Unsolicited::Exists(n),
            Untagged::Data(Data_::Expunge(n)) => Unsolicited::Expunge(n),
            Untagged::Data(Data_::Vanished { earlier, uids }) => Unsolicited::Vanished { earlier, uids },
            Untagged::Data(Data_::Fetch(f)) => Unsolicited::Fetch(f),
            Untagged::Data(Data_::Flags(f)) => Unsolicited::Flags(f),
            Untagged::Data(Data_::Capability(c)) => {
                self.caps = c;
                return false;
            }
            Untagged::Status(Done { code: Some(RespCode::Alert), text, .. }) => Unsolicited::Alert(text),
            _ => return false,
        };
        let mailbox = self.selected().map(|(m, _)| m.clone());
        self.notices.push_back(Notice { mailbox, event });
        true
    }

    fn need_selected(&self) -> Result<()> {
        match self.state {
            State::Selected { .. } => Ok(()),
            _ => Err(ImapError::State("no mailbox selected")),
        }
    }

    /// From now on, this session refuses every command that could change
    /// the mailbox (`SELECT`, `STORE`, `COPY`, `MOVE`, `EXPUNGE`, `APPEND`,
    /// `CREATE`), before it is sent: for an account the user wants read
    /// only, a mistake elsewhere in NiceMailer cannot reach the server.
    /// `EXAMINE` and every `PEEK` still work.
    pub fn forbid_writes(&mut self) {
        self.read_only = true;
    }

    fn need_writable(&self, command: &'static str) -> Result<()> {
        if self.read_only {
            return Err(ImapError::ReadOnly(command));
        }
        Ok(())
    }

    fn need_auth(&self) -> Result<()> {
        match self.state {
            State::NotAuthenticated => Err(ImapError::State("not logged in")),
            _ => Ok(()),
        }
    }

    // ---------------------------------------------------------------------
    // Connection and login
    // ---------------------------------------------------------------------

    async fn starttls(&mut self, spec: &ServerSpec) -> Result<()> {
        if self.state != State::NotAuthenticated {
            // PREAUTH over clear text: the server logged us in before any
            // protection; going on would be clear text.
            return Err(self.fail(ImapError::Tls("the server logged in before STARTTLS (PREAUTH)".into())));
        }
        let reply = self.run(Command::new("STARTTLS")).await?;
        if reply.done.kind != Kind::Ok {
            return Err(self.fail(ImapError::Tls(format!("the server refused STARTTLS: {}", reply.done.text))));
        }
        // Bytes already received after the OK were sent in clear text and
        // would be read as if protected (the "STARTTLS injection" of
        // CVE-2011-0411 and its kin): refuse them.
        if self.frag.progress().is_some() || !self.frag.message_bytes().is_empty() {
            return Err(self.fail(ImapError::Tls("the server sent data right after STARTTLS".into())));
        }
        let Some(Conn::Plain(tcp)) = self.conn.take() else {
            return Err(ImapError::Tls("STARTTLS on a connection that is not plain TCP".into()));
        };
        let connector = transport::connector(spec)?;
        let tls = transport::handshake(&connector, spec, tcp, &self.timeouts).await?;
        self.conn = Some(Conn::Tls(Box::new(tls)));
        self.frag = Fragmentizer::new(MAX_RESPONSE);
        self.caps = Capabilities::default();
        Ok(())
    }

    /// `CAPABILITY`: reads the capabilities again.
    pub async fn capability(&mut self) -> Result<&Capabilities> {
        let (_, untagged) = self.exec(Command::new("CAPABILITY")).await?;
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::Capability(c)) => self.caps = c,
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        Ok(&self.caps)
    }

    /// Logs in: `AUTHENTICATE PLAIN` if announced (any password, UTF-8
    /// included, travels without quoting), else `LOGIN`. Then reads the
    /// capabilities of the authenticated state, which are the ones that
    /// count (`plan-imap.md`, section 1): from the `[CAPABILITY]` code of the
    /// OK if the server gives it, else with a `CAPABILITY` command.
    ///
    /// The password is only written to the connection, never into an error
    /// or a log.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        match self.state {
            State::NotAuthenticated => {}
            // PREAUTH: already logged in by the server.
            _ => return Ok(()),
        }
        if !self.is_tls() && !matches!(self.conn, Some(Conn::Other(_))) && self.caps.has("LOGINDISABLED") && !self.caps.has("AUTH=PLAIN") {
            return Err(ImapError::Unsupported("a login over this connection (LOGINDISABLED)"));
        }
        let cmd = if self.caps.has("AUTH=PLAIN") {
            // RFC 4616: authorization id (empty), user, password, separated
            // by NUL bytes.
            let mut plain = Vec::with_capacity(username.len() + password.len() + 2);
            plain.push(0);
            plain.extend_from_slice(username.as_bytes());
            plain.push(0);
            plain.extend_from_slice(password.as_bytes());
            let b64 = encode::base64(&plain);
            if self.caps.has("SASL-IR") {
                Command::new("AUTHENTICATE").raw("PLAIN").raw(&b64)
            } else {
                Command::new("AUTHENTICATE").raw("PLAIN").wait().raw_tight(&b64)
            }
        } else if self.caps.has("LOGINDISABLED") {
            return Err(ImapError::Unsupported("a login mechanism NiceMailer knows (LOGIN or AUTH=PLAIN)"));
        } else {
            Command::new("LOGIN").astring(username.as_bytes()).astring(password.as_bytes())
        };
        let reply = self.run(cmd).await?;
        if self.malformed.is_none() && reply.done.kind != Kind::Ok {
            self.absorb(reply.untagged);
            // RFC 5530: these codes say the login is refused for another
            // reason than the password, for now.
            if let Some(RespCode::Other(code)) = &reply.done.code
                && matches!(code.as_str(), "UNAVAILABLE" | "LIMIT" | "INUSE")
            {
                return Err(ImapError::Unavailable { code: code.clone(), text: reply.done.text });
            }
            return Err(ImapError::AuthFailed(reply.done.text));
        }
        let code = reply.done.code.clone();
        let (_, untagged) = self.check("LOGIN", reply)?;
        self.state = State::Authenticated;
        let mut fresh = false;
        if let Some(RespCode::Capability(c)) = code {
            self.caps = c;
            fresh = true;
        }
        for u in untagged {
            if let Untagged::Data(Data_::Capability(c)) = u {
                self.caps = c;
                fresh = true;
            } else {
                self.absorb_one(u);
            }
        }
        if !fresh {
            self.capability().await?;
        }
        Ok(())
    }

    /// `ID` (RFC 2971): tells the server which client this is, if it asks
    /// for it. Some servers log it; none should depend on it.
    pub async fn id(&mut self) -> Result<()> {
        if !self.caps.has("ID") {
            return Ok(());
        }
        let cmd = Command::new("ID")
            .raw("(")
            .raw_tight("\"name\"")
            .string(b"NiceMailer")
            .string(b"version")
            .string(env!("CARGO_PKG_VERSION").as_bytes())
            .raw_tight(")");
        let (_, untagged) = self.exec(cmd).await?;
        self.absorb(untagged);
        Ok(())
    }

    /// `ENABLE` of the synchronization extensions the server announces:
    /// QRESYNC (which implies CONDSTORE, RFC 7162), or CONDSTORE alone.
    /// Returns the extensions now on.
    pub async fn enable_sync(&mut self) -> Result<Vec<String>> {
        self.need_auth()?;
        let wanted: Vec<&str> = ["CONDSTORE", "QRESYNC"].into_iter().filter(|c| self.caps.has(c)).collect();
        if wanted.is_empty() || !self.caps.has("ENABLE") {
            return Ok(self.enabled.iter().cloned().collect());
        }
        let (_, untagged) = self.exec(Command::new("ENABLE").raw(&wanted.join(" "))).await?;
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::Enabled(caps)) => self.enabled.extend(caps),
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        if self.enabled.contains("QRESYNC") {
            self.enabled.insert("CONDSTORE".into());
        }
        Ok(self.enabled.iter().cloned().collect())
    }

    /// `NAMESPACE` (RFC 2342), if announced.
    pub async fn namespace(&mut self) -> Result<Namespaces> {
        self.need_auth()?;
        if !self.caps.has("NAMESPACE") {
            return Err(ImapError::Unsupported("NAMESPACE"));
        }
        let (_, untagged) = self.exec(Command::new("NAMESPACE")).await?;
        let mut found = Namespaces::default();
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::Namespace(n)) => found = n,
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        Ok(found)
    }

    /// `NOOP`: lets the server report what changed, and checks that the
    /// connection is alive.
    pub async fn noop(&mut self) -> Result<()> {
        let (_, untagged) = self.exec(Command::new("NOOP")).await?;
        self.absorb(untagged);
        Ok(())
    }

    /// `LOGOUT`, then closes. Errors are ignored: the session ends anyway.
    pub async fn logout(&mut self) {
        if self.conn.is_some() {
            let _ = self.run(Command::new("LOGOUT")).await;
            if let Some(mut c) = self.conn.take() {
                let _ = tokio::time::timeout(Duration::from_secs(2), c.shutdown()).await;
            }
        }
    }

    // ---------------------------------------------------------------------
    // Mailboxes
    // ---------------------------------------------------------------------

    /// `LIST "" "*"`: every mailbox, with its special use (RFC 6154) when
    /// the server announces SPECIAL-USE.
    pub async fn list(&mut self) -> Result<Vec<ListEntry>> {
        Ok(self.list_inner(false).await?.into_iter().map(|(e, _)| e).collect())
    }

    /// `LIST` with the STATUS of each mailbox in the same answer
    /// (`LIST-STATUS`, RFC 5819), if announced.
    pub async fn list_with_status(&mut self) -> Result<Vec<(ListEntry, Option<MailboxStatus>)>> {
        if !self.caps.has("LIST-STATUS") {
            return Err(ImapError::Unsupported("LIST-STATUS"));
        }
        self.list_inner(true).await
    }

    async fn list_inner(&mut self, status: bool) -> Result<Vec<(ListEntry, Option<MailboxStatus>)>> {
        self.need_auth()?;
        let mut cmd = Command::new("LIST").string(b"").raw("\"*\"");
        let mut returns = Vec::new();
        // RFC 6154, section 2: the SPECIAL-USE return option goes with
        // LIST-EXTENDED (RFC 5258). Servers with SPECIAL-USE alone (and
        // Dovecot in any case) give the attributes in a plain LIST.
        if self.caps.has("SPECIAL-USE") && self.caps.has("LIST-EXTENDED") {
            returns.push("SPECIAL-USE".to_string());
        }
        if status {
            returns.push(format!("STATUS ({})", self.status_items()));
        }
        if !returns.is_empty() {
            cmd = cmd.raw(&format!("RETURN ({})", returns.join(" ")));
        }
        let (_, untagged) = self.exec(cmd).await?;
        let mut entries: Vec<(ListEntry, Option<MailboxStatus>)> = Vec::new();
        let mut statuses: Vec<(MailboxName, MailboxStatus)> = Vec::new();
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::List(e)) => entries.push((e, None)),
                Untagged::Data(Data_::Status(m, s)) => statuses.push((m, s)),
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        for (m, s) in statuses {
            if let Some(entry) = entries.iter_mut().find(|(e, _)| e.name.raw == m.raw) {
                entry.1 = Some(s);
            }
        }
        Ok(entries)
    }

    /// The items asked by `STATUS`: HIGHESTMODSEQ with CONDSTORE only.
    fn status_items(&self) -> &'static str {
        if self.caps.has("CONDSTORE") || self.caps.has("QRESYNC") {
            "MESSAGES UIDNEXT UIDVALIDITY UNSEEN HIGHESTMODSEQ"
        } else {
            "MESSAGES UIDNEXT UIDVALIDITY UNSEEN"
        }
    }

    /// `STATUS`: the counters of a mailbox without selecting it.
    pub async fn status(&mut self, mailbox: &MailboxName) -> Result<MailboxStatus> {
        self.need_auth()?;
        let items = self.status_items();
        let cmd = Command::new("STATUS").astring(&mailbox.raw).raw(&format!("({items})"));
        let (_, untagged) = self.exec(cmd).await?;
        let mut found = None;
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                // Names are compared as sent; only INBOX ignores case.
                Untagged::Data(Data_::Status(m, s)) if m.raw == mailbox.raw || (m.is_inbox() && mailbox.is_inbox()) => found = Some(s),
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        found.ok_or(ImapError::Malformed { command: "STATUS", detail: "no STATUS in the answer".into() })
    }

    /// `CREATE`. A mailbox that already exists is an error
    /// (`ImapError::No` with code `ALREADYEXISTS` on most servers).
    pub async fn create(&mut self, mailbox: &MailboxName) -> Result<()> {
        self.need_writable("CREATE")?;
        self.need_auth()?;
        let (_, untagged) = self.exec(Command::new("CREATE").astring(&mailbox.raw)).await?;
        self.absorb(untagged);
        Ok(())
    }

    /// `SELECT`: opens a mailbox for reading and writing. With QRESYNC on
    /// and `qresync` given, the server also says what changed since
    /// (`Selected::vanished`, `Selected::changed`).
    pub async fn select(&mut self, mailbox: &MailboxName, qresync: Option<&QresyncParams>) -> Result<Selected> {
        self.need_writable("SELECT")?;
        self.open_mailbox("SELECT", mailbox, qresync).await
    }

    /// `EXAMINE`: the same, read-only. Nothing NiceMailer does through it
    /// can change the mailbox, not even the `\Seen` flag.
    pub async fn examine(&mut self, mailbox: &MailboxName, qresync: Option<&QresyncParams>) -> Result<Selected> {
        self.open_mailbox("EXAMINE", mailbox, qresync).await
    }

    async fn open_mailbox(&mut self, name: &'static str, mailbox: &MailboxName, qresync: Option<&QresyncParams>) -> Result<Selected> {
        self.need_auth()?;
        let mut cmd = Command::new(name).astring(&mailbox.raw);
        let mut condstore_param = false;
        match qresync {
            Some(q) if self.qresync() => {
                let mut p = format!("(QRESYNC ({} {}", q.uid_validity, q.modseq.max(1));
                if let Some(known) = q.known_uids.as_ref().filter(|k| !k.is_empty() && k.to_string().len() <= MAX_SET_LEN) {
                    p.push_str(&format!(" {known}"));
                }
                p.push_str("))");
                cmd = cmd.raw(&p);
            }
            _ if !self.condstore() && self.caps.has("CONDSTORE") => {
                // Without ENABLE, CONDSTORE is turned on by the first
                // command that uses it (RFC 7162, 3.1).
                cmd = cmd.raw("(CONDSTORE)");
                condstore_param = true;
            }
            _ => {}
        }
        // Whatever the outcome, the previous mailbox is no longer selected
        // (RFC 3501, 6.3.1: a failed SELECT leaves none).
        self.state = State::Authenticated;
        let (done, untagged) = self.exec(cmd).await?;
        if condstore_param {
            self.enabled.insert("CONDSTORE".into());
        }
        let mut sel = Selected::default();
        let mut vanished: Vec<(u32, u32)> = Vec::new();
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::Exists(n)) => sel.exists = n,
                Untagged::Data(Data_::Flags(f)) => sel.flags = f,
                Untagged::Data(Data_::Vanished { earlier: true, uids }) => vanished.extend_from_slice(uids.ranges()),
                Untagged::Data(Data_::Fetch(f)) if qresync.is_some() && f.uid.is_some() => sel.changed.push(f),
                Untagged::Status(Done { code: Some(c), .. }) => match c {
                    RespCode::UidValidity(n) => sel.uid_validity = Some(n),
                    RespCode::UidNext(n) => sel.uid_next = Some(n),
                    RespCode::HighestModSeq(n) => sel.highest_modseq = Some(n),
                    RespCode::NoModSeq => sel.no_modseq = true,
                    RespCode::PermanentFlags(f) => sel.permanent_flags = f,
                    // [CLOSED]: the previous mailbox is closed (QRESYNC).
                    RespCode::Closed | RespCode::Other(_) | RespCode::ReadOnly | RespCode::ReadWrite => {}
                    other => rest.push(Untagged::Status(Done { kind: Kind::Ok, code: Some(other), text: String::new() })),
                },
                other => rest.push(other),
            }
        }
        sel.vanished = UidSet::from_ranges(vanished);
        sel.read_only = match done.code {
            Some(RespCode::ReadOnly) => true,
            Some(RespCode::ReadWrite) => false,
            _ => name == "EXAMINE",
        };
        self.state = State::Selected { mailbox: mailbox.clone(), read_only: sel.read_only };
        self.absorb(rest);
        Ok(sel)
    }

    // ---------------------------------------------------------------------
    // Messages
    // ---------------------------------------------------------------------

    /// `UID FETCH`. Returns one [`Fetched`] per message the server sent, in
    /// the order received. UIDs that no longer exist are silently absent
    /// (RFC 3501, 6.4.8). A set longer than a command line allows is split
    /// into several commands.
    pub async fn uid_fetch(&mut self, uids: &Uids, items: &FetchItems) -> Result<Vec<Fetched>> {
        Ok(self.uid_fetch_vanished(uids, items).await?.0)
    }

    /// `UID FETCH`, with the UIDs expunged since `items.changed_since` when
    /// `items.vanished` is asked (QRESYNC).
    pub async fn uid_fetch_vanished(&mut self, uids: &Uids, items: &FetchItems) -> Result<(Vec<Fetched>, UidSet)> {
        self.need_selected()?;
        if (items.modseq || items.changed_since.is_some()) && !self.condstore() {
            return Err(ImapError::Unsupported("CONDSTORE"));
        }
        if items.vanished && (!self.qresync() || items.changed_since.is_none()) {
            return Err(ImapError::Unsupported("QRESYNC (VANISHED needs CHANGEDSINCE)"));
        }
        let mut names = vec!["UID"];
        if items.flags {
            names.push("FLAGS");
        }
        if items.internal_date {
            names.push("INTERNALDATE");
        }
        if items.size {
            names.push("RFC822.SIZE");
        }
        if items.modseq {
            names.push("MODSEQ");
        }
        if items.header {
            names.push("BODY.PEEK[HEADER]");
        }
        let body;
        match items.body {
            Some(BodyRange::Whole) => names.push("BODY.PEEK[]"),
            Some(BodyRange::Partial { offset, len }) => {
                body = format!("BODY.PEEK[]<{offset}.{}>", len.max(1));
                names.push(&body);
            }
            None => {}
        }
        let mut modifiers = Vec::new();
        if let Some(m) = items.changed_since {
            modifiers.push(format!("CHANGEDSINCE {m}"));
        }
        if items.vanished {
            modifiers.push("VANISHED".into());
        }
        let list = format!("({})", names.join(" "));
        let mods = if modifiers.is_empty() { String::new() } else { format!("({})", modifiers.join(" ")) };

        let mut out = Vec::new();
        let mut vanished: Vec<(u32, u32)> = Vec::new();
        for set in set_texts(uids) {
            let mut cmd = Command::new("UID FETCH").raw(&set).raw(&list);
            if !mods.is_empty() {
                cmd = cmd.raw(&mods);
            }
            let (_, untagged) = self.exec(cmd).await?;
            let mut rest = Vec::new();
            for u in untagged {
                match u {
                    // A FETCH with a UID answers the command: the server
                    // includes the UID in every FETCH caused by a UID
                    // command (RFC 3501, 6.4.8). A flag change by another
                    // client may come in too: it is the current state as
                    // well, and is kept as a result.
                    Untagged::Data(Data_::Fetch(f)) if f.uid.is_some() => out.push(f),
                    Untagged::Data(Data_::Vanished { earlier: true, uids }) if items.vanished => vanished.extend_from_slice(uids.ranges()),
                    other => rest.push(other),
                }
            }
            self.absorb(rest);
        }
        Ok((out, UidSet::from_ranges(vanished)))
    }

    /// The body of message `uid`, given to `sink` as it arrives: in one
    /// piece up to [`WHOLE_BODY_MAX`] (`size` is the RFC822.SIZE known),
    /// else in pieces of [`BODY_CHUNK`], so that memory stays bounded
    /// whatever the size. Returns the number of bytes, or `None` if the
    /// message no longer exists.
    ///
    /// `sink` returns an error to stop (disk full…). That error is not an
    /// IMAP failure and leaves the session usable, so it comes back apart:
    /// the outer `Result` is the IMAP side, the inner one the sink's.
    pub async fn fetch_body<E>(&mut self, uid: u32, size: u32, sink: &mut dyn FnMut(&[u8]) -> std::result::Result<(), E>) -> Result<std::result::Result<Option<u64>, E>> {
        let one = Uids::Set(UidSet::from_uids([uid]));
        if size <= WHOLE_BODY_MAX {
            let items = FetchItems { body: Some(BodyRange::Whole), ..FetchItems::default() };
            let got = self.uid_fetch(&one, &items).await?;
            let Some(body) = got.into_iter().find(|f| f.uid == Some(uid)).and_then(|f| f.body) else { return Ok(Ok(None)) };
            if let Err(e) = sink(&body) {
                return Ok(Err(e));
            }
            return Ok(Ok(Some(body.len() as u64)));
        }
        let mut offset: u32 = 0;
        loop {
            let items = FetchItems { body: Some(BodyRange::Partial { offset, len: BODY_CHUNK }), ..FetchItems::default() };
            let got = self.uid_fetch(&one, &items).await?;
            let Some(piece) = got.into_iter().find(|f| f.uid == Some(uid)).and_then(|f| f.body) else {
                // Gone in the middle: what was written is incomplete.
                return Ok(Ok(None));
            };
            if !piece.is_empty()
                && let Err(e) = sink(&piece)
            {
                return Ok(Err(e));
            }
            let n = piece.len() as u32;
            offset = offset.saturating_add(n);
            // A short piece is the last one. The size announced is not
            // trusted to stop: some servers count it differently.
            if n < BODY_CHUNK || offset == u32::MAX {
                return Ok(Ok(Some(u64::from(offset))));
            }
        }
    }

    /// `UID SEARCH`: the UIDs matching, sorted.
    pub async fn uid_search(&mut self, key: &SearchKey) -> Result<Vec<u32>> {
        self.need_selected()?;
        let utf8 = search_needs_utf8(key);
        let mut cmd = Command::new("UID SEARCH");
        if utf8 {
            cmd = cmd.raw("CHARSET UTF-8");
        }
        cmd = push_key(cmd, key);
        let (_, untagged) = self.exec(cmd).await?;
        let mut found = Vec::new();
        let mut rest = Vec::new();
        for u in untagged {
            match u {
                Untagged::Data(Data_::Search(ids)) => found.extend(ids),
                other => rest.push(other),
            }
        }
        self.absorb(rest);
        found.sort_unstable();
        found.dedup();
        Ok(found)
    }

    /// `UID STORE`: changes flags. `silent`: the server does not send the
    /// new flags back. `unchanged_since` (CONDSTORE): leave alone the
    /// messages changed on the server after this modseq; they are listed in
    /// [`StoreResult::modified`].
    pub async fn uid_store(&mut self, uids: &Uids, op: StoreOp, flags: &[&str], silent: bool, unchanged_since: Option<u64>) -> Result<StoreResult> {
        self.need_writable("STORE")?;
        self.need_selected()?;
        if unchanged_since.is_some() && !self.condstore() {
            return Err(ImapError::Unsupported("CONDSTORE"));
        }
        for f in flags {
            if !valid_flag(f) {
                return Err(ImapError::State("invalid flag name"));
            }
        }
        let what = format!(
            "{}FLAGS{}",
            match op {
                StoreOp::Add => "+",
                StoreOp::Remove => "-",
                StoreOp::Replace => "",
            },
            if silent { ".SILENT" } else { "" }
        );
        let mut result = StoreResult::default();
        let mut modified: Vec<(u32, u32)> = Vec::new();
        for set in set_texts(uids) {
            let mut cmd = Command::new("UID STORE").raw(&set);
            if let Some(m) = unchanged_since {
                cmd = cmd.raw(&format!("(UNCHANGEDSINCE {m})"));
            }
            cmd = cmd.raw(&what).raw(&format!("({})", flags.join(" ")));
            let (done, untagged) = self.exec(cmd).await?;
            if let Some(RespCode::Modified(m)) = done.code {
                modified.extend_from_slice(m.ranges());
            }
            let mut rest = Vec::new();
            for u in untagged {
                match u {
                    Untagged::Data(Data_::Fetch(f)) if f.uid.is_some() => result.fetched.push(f),
                    other => rest.push(other),
                }
            }
            self.absorb(rest);
        }
        result.modified = UidSet::from_ranges(modified);
        Ok(result)
    }

    /// `UID COPY`: returns where the messages went, with `UIDPLUS`.
    pub async fn uid_copy(&mut self, uids: &Uids, to: &MailboxName) -> Result<Option<CopyUids>> {
        self.need_writable("COPY")?;
        self.need_selected()?;
        let mut all: Option<CopyUids> = None;
        for set in set_texts(uids) {
            let (done, untagged) = self.exec(Command::new("UID COPY").raw(&set).astring(&to.raw)).await?;
            self.absorb(untagged);
            if let Some(RespCode::CopyUid(c)) = done.code {
                merge_copy(&mut all, c);
            }
        }
        Ok(all)
    }

    /// Moves messages: `UID MOVE` (RFC 6851) if announced; else `UID COPY`,
    /// `\Deleted`, then `UID EXPUNGE` of those UIDs only (UIDPLUS). Without
    /// UIDPLUS either, the messages stay flagged `\Deleted`
    /// ([`MoveResult::expunged`]). The expunges come as notices.
    pub async fn uid_move(&mut self, uids: &Uids, to: &MailboxName) -> Result<MoveResult> {
        self.need_writable("MOVE")?;
        self.need_selected()?;
        if !self.caps.has("MOVE") {
            let copy_uids = self.uid_copy(uids, to).await?;
            self.uid_store(uids, StoreOp::Add, &["\\Deleted"], true, None).await?;
            if !self.caps.has("UIDPLUS") {
                return Ok(MoveResult { copy_uids, expunged: false });
            }
            self.uid_expunge(uids).await?;
            return Ok(MoveResult { copy_uids, expunged: true });
        }
        let mut all: Option<CopyUids> = None;
        for set in set_texts(uids) {
            let (done, untagged) = self.exec(Command::new("UID MOVE").raw(&set).astring(&to.raw)).await?;
            // RFC 6851, 4.3: COPYUID comes in an untagged OK, before the
            // expunges; some servers put it in the tagged OK.
            let mut rest = Vec::new();
            for u in untagged {
                match u {
                    Untagged::Status(Done { code: Some(RespCode::CopyUid(c)), .. }) => merge_copy(&mut all, c),
                    other => rest.push(other),
                }
            }
            if let Some(RespCode::CopyUid(c)) = done.code {
                merge_copy(&mut all, c);
            }
            self.absorb(rest);
        }
        Ok(MoveResult { copy_uids: all, expunged: true })
    }

    /// `UID EXPUNGE` (UIDPLUS): expunges these UIDs only, if flagged
    /// `\Deleted`. The expunges come as notices.
    pub async fn uid_expunge(&mut self, uids: &Uids) -> Result<()> {
        self.need_writable("EXPUNGE")?;
        self.need_selected()?;
        if !self.caps.has("UIDPLUS") {
            return Err(ImapError::Unsupported("UIDPLUS"));
        }
        for set in set_texts(uids) {
            let (_, untagged) = self.exec(Command::new("UID EXPUNGE").raw(&set)).await?;
            self.absorb(untagged);
        }
        Ok(())
    }

    /// `APPEND`: adds a message to a mailbox. `internal_date`: Unix
    /// seconds, the date of arrival to give it (the server's clock
    /// otherwise). Returns its UID, with UIDPLUS.
    pub async fn append(&mut self, mailbox: &MailboxName, flags: &[&str], internal_date: Option<i64>, message: Vec<u8>) -> Result<Option<AppendUid>> {
        self.need_writable("APPEND")?;
        self.need_auth()?;
        for f in flags {
            if !valid_flag(f) || f.eq_ignore_ascii_case("\\Recent") {
                return Err(ImapError::State("invalid flag name"));
            }
        }
        let mut cmd = Command::new("APPEND").astring(&mailbox.raw);
        if !flags.is_empty() {
            cmd = cmd.raw(&format!("({})", flags.join(" ")));
        }
        if let Some(t) = internal_date {
            cmd = cmd.raw(&format!("\"{}\"", imap_date_time(t)));
        }
        let (done, untagged) = self.exec(cmd.literal(message)).await?;
        self.absorb(untagged);
        Ok(match done.code {
            Some(RespCode::AppendUid(a)) => Some(a),
            _ => None,
        })
    }

    /// `IDLE` (RFC 2177): waits for the server to report something on the
    /// selected mailbox, for at most `max` (capped at [`IDLE_MAX`]), or
    /// until `stop` completes. What was reported is in
    /// [`Session::take_unsolicited`].
    ///
    /// The server's keep-alive lines (Dovecot's `* OK Still here`) do not
    /// end it.
    pub async fn idle<F: Future<Output = ()>>(&mut self, max: Duration, stop: F) -> Result<IdleEnd> {
        let r = self.idle_inner(max, stop).await;
        r.map_err(|e| self.fail(e))
    }

    async fn idle_inner<F: Future<Output = ()>>(&mut self, max: Duration, stop: F) -> Result<IdleEnd> {
        self.need_selected()?;
        if !self.caps.has("IDLE") {
            return Err(ImapError::Unsupported("IDLE"));
        }
        if !self.notices.is_empty() {
            return Ok(IdleEnd::Changes);
        }
        if self.conn.is_none() {
            return Err(ImapError::Closed);
        }
        self.malformed = None;
        let tag = self.tag();
        self.write(format!("{tag} IDLE\r\n").as_bytes()).await?;
        let mut untagged = Vec::new();
        if let Some(done) = self.wait_continue(&tag, &mut untagged).await? {
            let reply = Reply { untagged, done };
            let (_, rest) = self.check("IDLE", reply)?;
            self.absorb(rest);
            return Ok(IdleEnd::Changes);
        }
        self.absorb(untagged);

        let deadline = tokio::time::Instant::now() + max.min(IDLE_MAX);
        // Rust note: `pin!` fixes the future in place, so that `select!`
        // can poll it again at each turn of the loop without moving it.
        let stop = std::pin::pin!(stop);
        let mut stop = stop;
        let end = loop {
            tokio::select! {
                r = self.read_resp(None) => match r? {
                    Ok(Resp::Data(d)) => {
                        if self.absorb_one(Untagged::Data(d)) {
                            break IdleEnd::Changes;
                        }
                    }
                    Ok(Resp::Status(s)) => {
                        self.absorb_one(Untagged::Status(s));
                    }
                    Ok(Resp::Bye(text)) => self.bye = Some(text),
                    Ok(Resp::Tagged { tag: t, status }) if t == tag => {
                        // The server ended IDLE itself.
                        self.check("IDLE", Reply { untagged: Vec::new(), done: status })?;
                        return Ok(IdleEnd::Timeout);
                    }
                    Ok(other) => return Err(ImapError::Protocol(format!("unexpected response during IDLE: {other:?}"))),
                    Err((_, detail)) => {
                        self.malformed.get_or_insert(detail);
                        break IdleEnd::Changes;
                    }
                },
                _ = &mut stop => break IdleEnd::Stopped,
                _ = tokio::time::sleep_until(deadline) => break IdleEnd::Timeout,
            }
        };
        self.write(b"DONE\r\n").await?;
        let mut untagged = Vec::new();
        loop {
            match self.next_for(&tag, &mut untagged).await? {
                Step::Done(done) => {
                    let (_, rest) = self.check("IDLE", Reply { untagged, done })?;
                    self.absorb(rest);
                    return Ok(end);
                }
                Step::Continue => return Err(ImapError::Protocol("unexpected '+' after DONE".into())),
                Step::More => {}
            }
        }
    }
}

/// One response read for a command.
enum Step {
    Continue,
    Done(Done),
    More,
}

/// Name of a response code, for [`ImapError::No`].
fn code_name(c: RespCode) -> String {
    match c {
        RespCode::Other(s) => s,
        other => format!("{other:?}"),
    }
}

/// The texts of a set of UIDs, split to fit a command line.
fn set_texts(uids: &Uids) -> Vec<String> {
    match uids {
        Uids::From(n) => vec![format!("{}:*", (*n).max(1))],
        Uids::Set(s) => s.chunks(MAX_SET_LEN).iter().map(UidSet::to_string).collect(),
    }
}

fn merge_copy(all: &mut Option<CopyUids>, c: CopyUids) {
    match all {
        Some(a) if a.uid_validity == c.uid_validity => a.pairs.extend(c.pairs),
        _ => *all = Some(c),
    }
}

/// A flag NiceMailer may send: a system flag (`\Seen`) or a keyword atom
/// (`$Junk`, `$NmWasTodo`).
fn valid_flag(f: &str) -> bool {
    let name = f.strip_prefix('\\').unwrap_or(f);
    !name.is_empty() && name.bytes().all(|b| (0x21..=0x7e).contains(&b) && !b"(){%*\"\\]".contains(&b))
}

fn search_needs_utf8(key: &SearchKey) -> bool {
    match key {
        SearchKey::Header(n, v) => !n.is_ascii() || !v.is_ascii(),
        SearchKey::And(a, b) => search_needs_utf8(a) || search_needs_utf8(b),
        SearchKey::Not(k) => search_needs_utf8(k),
        _ => false,
    }
}

fn push_key(cmd: Command, key: &SearchKey) -> Command {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    match key {
        SearchKey::All => cmd.raw("ALL"),
        SearchKey::Unseen => cmd.raw("UNSEEN"),
        SearchKey::Seen => cmd.raw("SEEN"),
        SearchKey::Flagged => cmd.raw("FLAGGED"),
        SearchKey::Deleted => cmd.raw("DELETED"),
        SearchKey::Before { year, month, day } => {
            let m = MONTHS[((*month).clamp(1, 12) - 1) as usize];
            cmd.raw(&format!("BEFORE {day}-{m}-{year}"))
        }
        SearchKey::Not(k) => push_key(cmd.raw("NOT"), k),
        // An empty set matches nothing; `UID 0` is not valid, and `NOT ALL`
        // says it plainly.
        SearchKey::Uid(s) if s.is_empty() => cmd.raw("NOT ALL"),
        SearchKey::Uid(s) => cmd.raw("UID").raw(&s.to_string()),
        SearchKey::Since { year, month, day } => {
            let m = MONTHS[((*month).clamp(1, 12) - 1) as usize];
            cmd.raw(&format!("SINCE {day}-{m}-{year}"))
        }
        SearchKey::Header(name, value) => cmd.raw("HEADER").astring(name.as_bytes()).astring(value.as_bytes()),
        SearchKey::And(a, b) => push_key(push_key(cmd, a), b),
    }
}

/// A date-time of `APPEND` (RFC 3501, `date-time`): `"05-Nov-2026 14:03:00
/// +0000"`, in UTC.
pub(crate) fn imap_date_time(unix: i64) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm,
    // valid for any date of the proleptic Gregorian calendar).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{day:02}-{}-{year:04} {:02}:{:02}:{:02} +0000",
        MONTHS[(month - 1) as usize],
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}


/// A copy of an untagged response without its `UIDVALIDITY 0` (see
/// `read_resp`), or `None` if it has none. Only two shapes are touched, so
/// that the bytes of a mail (a literal) are never changed:
///
/// - `* STATUS <mailbox> (… UIDVALIDITY 0 …)`: the item leaves the list,
///   which is the last parenthesized group of the line (a mailbox name in
///   a literal comes before it);
/// - `* OK [UIDVALIDITY 0] text`, in the answer to `SELECT` or `EXAMINE`:
///   the code leaves the response.
fn without_zero_uidvalidity(bytes: &[u8]) -> Option<Vec<u8>> {
    let starts = |prefix: &[u8]| bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix);
    if starts(b"* OK [UIDVALIDITY 0]") {
        let rest = &bytes[b"* OK [UIDVALIDITY 0]".len()..];
        let rest = rest.strip_prefix(b" ").unwrap_or(rest);
        return Some([b"* OK ".as_slice(), rest].concat());
    }
    if !starts(b"* STATUS ") {
        return None;
    }
    let open = bytes.iter().rposition(|&b| b == b'(')?;
    let close = open + bytes[open..].iter().position(|&b| b == b')')?;
    let text = std::str::from_utf8(&bytes[open + 1..close]).ok()?;
    let words: Vec<&str> = text.split_ascii_whitespace().collect();
    if !words.len().is_multiple_of(2) {
        return None;
    }
    let mut kept = Vec::new();
    let mut dropped = false;
    for pair in words.chunks(2) {
        if pair[0].eq_ignore_ascii_case("UIDVALIDITY") && pair[1] == "0" {
            dropped = true;
        } else {
            kept.extend_from_slice(pair);
        }
    }
    dropped.then(|| [&bytes[..=open], kept.join(" ").as_bytes(), &bytes[close..]].concat())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_of_append() {
        assert_eq!(imap_date_time(0), "01-Jan-1970 00:00:00 +0000");
        assert_eq!(imap_date_time(1_793_887_380), "05-Nov-2026 14:03:00 +0000");
        assert_eq!(imap_date_time(951_782_400), "29-Feb-2000 00:00:00 +0000");
        assert_eq!(imap_date_time(-1), "31-Dec-1969 23:59:59 +0000");
    }

    #[test]
    fn flags_are_checked() {
        for ok in ["\\Seen", "\\Deleted", "$Junk", "$NmWasTodo", "NonJunk"] {
            assert!(valid_flag(ok), "{ok}");
        }
        for bad in ["", "\\", "a b", "a)", "\\Seen\r\nA1 LOGOUT", "x*", "é"] {
            assert!(!valid_flag(bad), "{bad:?}");
        }
    }

    #[test]
    fn long_sets_are_split() {
        let scattered = UidSet::from_uids((1..20_000).map(|i| i * 2));
        let texts = set_texts(&Uids::Set(scattered.clone()));
        assert!(texts.len() > 1);
        assert!(texts.iter().all(|t| t.len() <= MAX_SET_LEN), "{:?}", texts.iter().map(String::len).max());
        let rejoined: Vec<u32> = texts
            .iter()
            .flat_map(|t| t.split(',').map(|n| n.parse::<u32>().unwrap()).collect::<Vec<_>>())
            .collect();
        assert_eq!(rejoined, scattered.iter().collect::<Vec<_>>());
        assert_eq!(set_texts(&Uids::From(0)), vec!["1:*"]);
    }

    #[test]
    fn search_keys() {
        let c = |k: &SearchKey| String::from_utf8(push_key(Command::new("UID SEARCH"), k).flatten()).unwrap();
        assert_eq!(c(&SearchKey::All), "UID SEARCH ALL");
        assert_eq!(c(&SearchKey::Uid(UidSet::from_uids([1, 2, 3, 7]))), "UID SEARCH UID 1:3,7");
        assert_eq!(c(&SearchKey::Uid(UidSet::default())), "UID SEARCH NOT ALL");
        assert_eq!(c(&SearchKey::Since { year: 2026, month: 11, day: 5 }), "UID SEARCH SINCE 5-Nov-2026");
        assert_eq!(
            c(&SearchKey::Header("Message-ID".into(), "<a b@x>".into())),
            "UID SEARCH HEADER Message-ID \"<a b@x>\""
        );
        let archive = SearchKey::And(
            Box::new(SearchKey::Not(Box::new(SearchKey::Flagged))),
            Box::new(SearchKey::And(Box::new(SearchKey::Seen), Box::new(SearchKey::Before { year: 2026, month: 10, day: 6 }))),
        );
        assert_eq!(c(&archive), "UID SEARCH NOT FLAGGED SEEN BEFORE 6-Oct-2026");
        let and = SearchKey::And(Box::new(SearchKey::Unseen), Box::new(SearchKey::Deleted));
        assert_eq!(c(&and), "UID SEARCH UNSEEN DELETED");
        assert!(search_needs_utf8(&SearchKey::Header("Subject".into(), "été".into())));
    }

    #[test]
    fn a_zero_uidvalidity_is_taken_out_and_nothing_else() {
        let f = |b: &[u8]| without_zero_uidvalidity(b).map(|v| String::from_utf8(v).unwrap());
        assert_eq!(
            f(b"* STATUS Drafts (MESSAGES 0 UIDNEXT 1 UIDVALIDITY 0 UNSEEN 0 HIGHESTMODSEQ 1)\r\n").as_deref(),
            Some("* STATUS Drafts (MESSAGES 0 UIDNEXT 1 UNSEEN 0 HIGHESTMODSEQ 1)\r\n")
        );
        assert_eq!(f(b"* status \"A (b)\" (uidvalidity 0)\r\n").as_deref(), Some("* status \"A (b)\" ()\r\n"));
        assert_eq!(f(b"* OK [UIDVALIDITY 0] UIDs valid\r\n").as_deref(), Some("* OK UIDs valid\r\n"));
        // Valid values, and other responses, are left alone.
        assert_eq!(f(b"* STATUS Drafts (UIDVALIDITY 10 UIDNEXT 0)\r\n"), None);
        assert_eq!(f(b"* 1 FETCH (BODY[] {13}\r\nUIDVALIDITY 0)\r\n"), None);
        assert_eq!(f(b"* OK [UIDVALIDITY 07] x\r\n"), None);
    }
}
