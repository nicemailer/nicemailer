//! The SMTP client (`specifications/reponse.md`, section 7, "SMTP"): what
//! sending one mail takes, nothing more.
//!
//! ```text
//! connect ─▶ 220 ─▶ EHLO ─▶ [STARTTLS ─▶ EHLO] ─▶ AUTH ─▶ MAIL FROM ─▶ RCPT TO… ─▶ DATA ─▶ QUIT
//! ```
//!
//! It runs over the transport of the IMAP client (`imap/transport.rs`):
//! the same TLS, with certificates checked by macOS and the optional
//! authority of the account, the same refusal of clear text beyond this
//! Mac.
//!
//! # Failures
//!
//! [`SmtpError::is_permanent`] tells what trying again cannot fix: a
//! refused login, a `5xx` answer, a message larger than the server
//! accepts, no recipient accepted. The rest (network, time limit, a `4xx`
//! answer such as "try again later") is temporary: the caller sends again
//! later (`docs/sending.md`).
//!
//! # What is never trusted
//!
//! - Addresses go into the envelope only if [`nm_core::compose::valid_addr`]
//!   accepts them: no space, no `>`, no line break, so a hostile header
//!   cannot close `RCPT TO:<…>` and add a command.
//! - Bytes received right after the server agreed to `STARTTLS` were sent
//!   in clear text and are refused (the "STARTTLS injection").
//! - The password is only sent over TLS, or to this Mac.

use std::time::Duration;

use nm_core::compose::{base64, valid_addr};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::imap::transport::{self, Conn};
use crate::imap::{ImapError, Security, ServerSpec, Stream, Timeouts};

/// Longest line of a reply read; a server sending more is broken.
const MAX_LINE: usize = 4096;
/// Most lines of one (multi-line) reply.
const MAX_LINES: usize = 200;

/// An SMTP failure. The texts reach the user (the status bar, `SendFailed`),
/// in English like the CLI; none holds the password.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SmtpError {
    #[error("cannot connect to {0}")]
    Connect(String),
    #[error("TLS: {0}")]
    Tls(String),
    #[error("connection lost: {0}")]
    Io(String),
    #[error("the server did not answer in time ({0})")]
    Timeout(&'static str),
    #[error("the server closed the connection")]
    Closed,
    #[error("protocol error: {0}")]
    Protocol(String),
    /// The server refused the user name or the password (`535`), or offers
    /// no way to log in that NiceMailer knows.
    #[error("the server refused the login: {0}")]
    Auth(String),
    /// The server answered a command with an error code.
    #[error("{stage} refused by the server: {code} {text}")]
    Rejected { stage: &'static str, code: u16, text: String },
    /// Larger than the server accepts (`SIZE`): not even tried.
    #[error("the message is {size} bytes, the server accepts at most {max}")]
    TooLarge { size: u64, max: u64 },
    /// Every recipient was refused, with the server's answer for each.
    #[error("no recipient accepted: {}", .0.iter().map(|r| format!("{} ({} {})", r.addr, r.code, r.text)).collect::<Vec<_>>().join(", "))]
    NoRecipient(Vec<Refused>),
    /// An address that cannot go into the envelope.
    #[error("invalid address: {0}")]
    InvalidAddress(String),
}

impl SmtpError {
    /// Whether trying again later cannot help (`reponse.md`, section 7,
    /// "Résultat de l'envoi"): the draft goes back to the user.
    pub fn is_permanent(&self) -> bool {
        match self {
            SmtpError::Auth(_) | SmtpError::TooLarge { .. } | SmtpError::InvalidAddress(_) => true,
            SmtpError::Rejected { code, .. } => *code >= 500,
            // A temporary refusal of one recipient (a full mailbox, a
            // greylisting server) may pass later.
            SmtpError::NoRecipient(r) => r.iter().all(|r| r.code >= 500),
            _ => false,
        }
    }
}

impl From<ImapError> for SmtpError {
    /// The errors of the shared transport.
    fn from(e: ImapError) -> SmtpError {
        match e {
            ImapError::Connect(s) => SmtpError::Connect(s),
            ImapError::Tls(s) => SmtpError::Tls(s),
            ImapError::Timeout(s) => SmtpError::Timeout(s),
            ImapError::Io(s) => SmtpError::Io(s),
            other => SmtpError::Protocol(other.to_string()),
        }
    }
}

pub type Result<T> = std::result::Result<T, SmtpError>;

/// A recipient the server refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub addr: String,
    pub code: u16,
    pub text: String,
}

/// What the server accepted: the mail went to every recipient but those
/// of `refused` (`reponse.md`, section 7: sending to the others).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sent {
    pub refused: Vec<Refused>,
}

/// A reply of the server: its code and its text, lines joined by spaces.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reply {
    code: u16,
    text: String,
    /// Each line's text, for the extensions of `EHLO`.
    lines: Vec<String>,
}

/// What the server announced after `EHLO`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extensions {
    pub starttls: bool,
    /// Mechanisms of `AUTH`, upper case.
    pub auth: Vec<String>,
    /// Largest message accepted (`SIZE`), 0 or absent for no limit.
    pub size: Option<u64>,
}

impl Extensions {
    fn parse(lines: &[String]) -> Extensions {
        let mut e = Extensions::default();
        // The first line is the server's name.
        for l in lines.iter().skip(1) {
            let mut words = l.split_whitespace();
            let Some(kw) = words.next() else { continue };
            match kw.to_ascii_uppercase().as_str() {
                "STARTTLS" => e.starttls = true,
                "AUTH" => e.auth.extend(words.map(|w| w.to_ascii_uppercase())),
                "SIZE" => e.size = words.next().and_then(|n| n.parse().ok()).filter(|n| *n > 0),
                _ => {}
            }
        }
        e
    }
}

/// One connection to an SMTP server.
pub struct Client {
    conn: Option<Conn>,
    /// Bytes read and not used yet.
    pending: Vec<u8>,
    timeouts: Timeouts,
    ext: Extensions,
    /// TLS, or a connection to this Mac: the password may be sent.
    secure: bool,
    /// The name given in `EHLO`.
    helo: String,
}

impl Client {
    /// Connects, reads the greeting, says `EHLO`, and upgrades to TLS for
    /// [`Security::Starttls`] (refused if the server cannot: never a
    /// fallback to clear text).
    pub async fn connect(spec: &ServerSpec, timeouts: Timeouts) -> Result<Client> {
        let conn = transport::open(spec, &timeouts).await?;
        let secure = conn.is_tls() || transport::is_loopback(&spec.host);
        let mut c = Client::new(conn, timeouts, secure);
        c.greeting().await?;
        c.ehlo().await?;
        if spec.security == Security::Starttls {
            c.starttls(spec).await?;
        }
        Ok(c)
    }

    /// A client over a stream already open (tests: a scripted server).
    /// `secure`: whether to consider it protected.
    pub async fn over(stream: impl Stream + 'static, timeouts: Timeouts, secure: bool) -> Result<Client> {
        let mut c = Client::new(Conn::Other(Box::new(stream)), timeouts, secure);
        c.greeting().await?;
        c.ehlo().await?;
        Ok(c)
    }

    fn new(conn: Conn, timeouts: Timeouts, secure: bool) -> Client {
        // A name that says nothing of the Mac: the host name would tell
        // every server who sends.
        Client { conn: Some(conn), pending: Vec::new(), timeouts, ext: Extensions::default(), secure, helo: "localhost".into() }
    }

    pub fn extensions(&self) -> &Extensions {
        &self.ext
    }

    async fn greeting(&mut self) -> Result<()> {
        let r = self.read_reply().await?;
        if r.code != 220 {
            return Err(SmtpError::Rejected { stage: "connection", code: r.code, text: r.text });
        }
        Ok(())
    }

    async fn ehlo(&mut self) -> Result<()> {
        let r = self.command(&format!("EHLO {}", self.helo)).await?;
        if r.code != 250 {
            return Err(SmtpError::Rejected { stage: "EHLO", code: r.code, text: r.text });
        }
        self.ext = Extensions::parse(&r.lines);
        Ok(())
    }

    async fn starttls(&mut self, spec: &ServerSpec) -> Result<()> {
        if !self.ext.starttls {
            return Err(SmtpError::Tls("the server does not offer STARTTLS".into()));
        }
        let r = self.command("STARTTLS").await?;
        if r.code != 220 {
            return Err(SmtpError::Tls(format!("the server refused STARTTLS: {} {}", r.code, r.text)));
        }
        if !self.pending.is_empty() {
            self.conn = None;
            return Err(SmtpError::Tls("the server sent data right after STARTTLS".into()));
        }
        let Some(Conn::Plain(tcp)) = self.conn.take() else {
            return Err(SmtpError::Tls("STARTTLS on a connection that is not plain TCP".into()));
        };
        let connector = transport::connector(spec)?;
        let tls = transport::handshake(&connector, spec, tcp, &self.timeouts).await?;
        self.conn = Some(Conn::Tls(Box::new(tls)));
        self.secure = true;
        // What the server said before TLS may have been changed by anyone
        // on the network: asked again (RFC 3207, 4.2).
        self.ext = Extensions::default();
        self.ehlo().await
    }

    /// Logs in with `PLAIN`, else `LOGIN`. A server that announces no
    /// `AUTH` (a relay on this Mac, a test server) is used without login.
    pub async fn login(&mut self, username: &str, password: &str) -> Result<()> {
        if self.ext.auth.is_empty() {
            return Ok(());
        }
        if !self.secure {
            return Err(SmtpError::Tls("refusing to send the password over a connection without TLS".into()));
        }
        let r = if self.ext.auth.iter().any(|m| m == "PLAIN") {
            let token = base64(format!("\0{username}\0{password}").as_bytes());
            self.command_secret(&format!("AUTH PLAIN {token}")).await?
        } else if self.ext.auth.iter().any(|m| m == "LOGIN") {
            let r = self.command("AUTH LOGIN").await?;
            if r.code != 334 {
                return Err(auth_error(r));
            }
            let r = self.command_secret(&base64(username.as_bytes())).await?;
            if r.code != 334 {
                return Err(auth_error(r));
            }
            self.command_secret(&base64(password.as_bytes())).await?
        } else {
            return Err(SmtpError::Auth(format!("no login method NiceMailer knows (the server offers {})", self.ext.auth.join(" "))));
        };
        if r.code != 235 {
            return Err(auth_error(r));
        }
        Ok(())
    }

    /// Sends one message to `to` (every recipient: To, Cc and Bcc), from
    /// `from`. `data` is the MIME message, lines ended by CRLF.
    ///
    /// `before_end` is called just before the final `.` that tells the
    /// server the message is complete: from then on, the server may have
    /// sent it even if no answer comes back (a connection lost), and the
    /// caller must not simply send again (`reponse.md`, "Double envoi").
    ///
    /// Recipients refused while others are accepted are reported in
    /// [`Sent::refused`]; all refused is [`SmtpError::NoRecipient`].
    pub async fn send(&mut self, from: &str, to: &[String], data: &[u8], before_end: impl FnOnce()) -> Result<Sent> {
        for a in std::iter::once(from).chain(to.iter().map(String::as_str)) {
            if !valid_addr(a) {
                return Err(SmtpError::InvalidAddress(a.chars().filter(|c| !c.is_control()).collect()));
            }
        }
        if to.is_empty() {
            return Err(SmtpError::NoRecipient(Vec::new()));
        }
        let size = data.len() as u64;
        if let Some(max) = self.ext.size
            && size > max
        {
            return Err(SmtpError::TooLarge { size, max });
        }
        let mail = if self.ext.size.is_some() { format!("MAIL FROM:<{from}> SIZE={size}") } else { format!("MAIL FROM:<{from}>") };
        let r = self.command(&mail).await?;
        if r.code != 250 {
            return Err(SmtpError::Rejected { stage: "MAIL FROM", code: r.code, text: r.text });
        }
        let mut sent = Sent::default();
        for rcpt in to {
            let r = self.command(&format!("RCPT TO:<{rcpt}>")).await?;
            if r.code != 250 && r.code != 251 {
                sent.refused.push(Refused { addr: rcpt.clone(), code: r.code, text: r.text });
            }
        }
        if sent.refused.len() == to.len() {
            // Nothing will be sent: the transaction is cancelled politely.
            let _ = self.command("RSET").await;
            return Err(SmtpError::NoRecipient(sent.refused));
        }
        let r = self.command("DATA").await?;
        if r.code != 354 {
            return Err(SmtpError::Rejected { stage: "DATA", code: r.code, text: r.text });
        }
        self.write(&dot_stuff(data)).await?;
        before_end();
        self.write(b".\r\n").await?;
        let r = self.read_reply().await?;
        if r.code != 250 {
            return Err(SmtpError::Rejected { stage: "DATA", code: r.code, text: r.text });
        }
        Ok(sent)
    }

    /// `QUIT`, without waiting long: the mail is already sent.
    pub async fn quit(mut self) {
        let _ = tokio::time::timeout(Duration::from_secs(5), self.command("QUIT")).await;
    }

    async fn command(&mut self, line: &str) -> Result<Reply> {
        self.write(format!("{line}\r\n").as_bytes()).await?;
        self.read_reply().await
    }

    /// The same, for a line holding the password: an error never shows it.
    async fn command_secret(&mut self, line: &str) -> Result<Reply> {
        self.command(line).await.map_err(|e| match e {
            SmtpError::Protocol(_) => SmtpError::Protocol("unreadable answer to the login".into()),
            other => other,
        })
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        let io = self.timeouts.io;
        let conn = self.conn.as_mut().ok_or(SmtpError::Closed)?;
        let r = tokio::time::timeout(io, async {
            conn.write_all(bytes).await?;
            conn.flush().await
        })
        .await;
        match r {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => {
                self.conn = None;
                Err(SmtpError::Io(e.to_string()))
            }
            Err(_) => {
                self.conn = None;
                Err(SmtpError::Timeout("sending"))
            }
        }
    }

    /// Reads one reply, multi-line included (`250-…` lines, then `250 …`).
    async fn read_reply(&mut self) -> Result<Reply> {
        let mut lines = Vec::new();
        let mut code = 0;
        loop {
            let line = self.read_line().await?;
            let bytes = line.as_bytes();
            if bytes.len() < 3 || !bytes[..3].iter().all(u8::is_ascii_digit) || (bytes.len() > 3 && bytes[3] != b' ' && bytes[3] != b'-') {
                self.conn = None;
                return Err(SmtpError::Protocol(format!("unexpected line from the server: {}", line.chars().take(80).collect::<String>())));
            }
            let c: u16 = line[..3].parse().unwrap_or(0);
            if code != 0 && c != code {
                self.conn = None;
                return Err(SmtpError::Protocol("reply lines with different codes".into()));
            }
            code = c;
            lines.push(line.get(4..).unwrap_or("").to_string());
            if bytes.len() == 3 || bytes[3] == b' ' {
                break;
            }
            if lines.len() >= MAX_LINES {
                self.conn = None;
                return Err(SmtpError::Protocol("reply too long".into()));
            }
        }
        if code == 421 {
            // "Service not available, closing": the server leaves.
            self.conn = None;
        }
        Ok(Reply { code, text: lines.join(" ").trim().to_string(), lines })
    }

    async fn read_line(&mut self) -> Result<String> {
        loop {
            if let Some(end) = self.pending.windows(2).position(|w| w == b"\r\n") {
                let line: Vec<u8> = self.pending.drain(..end + 2).collect();
                return Ok(String::from_utf8_lossy(&line[..end]).into_owned());
            }
            if self.pending.len() > MAX_LINE {
                self.conn = None;
                return Err(SmtpError::Protocol("line too long".into()));
            }
            let io = self.timeouts.io;
            let conn = self.conn.as_mut().ok_or(SmtpError::Closed)?;
            let mut buf = [0u8; 4096];
            match tokio::time::timeout(io, conn.read(&mut buf)).await {
                Ok(Ok(0)) => {
                    self.conn = None;
                    return Err(SmtpError::Closed);
                }
                Ok(Ok(n)) => self.pending.extend_from_slice(&buf[..n]),
                Ok(Err(e)) => {
                    self.conn = None;
                    return Err(SmtpError::Io(e.to_string()));
                }
                Err(_) => {
                    self.conn = None;
                    return Err(SmtpError::Timeout("waiting for the server"));
                }
            }
        }
    }
}

fn auth_error(r: Reply) -> SmtpError {
    if (400..500).contains(&r.code) {
        // "Temporary authentication failure": the server's database is
        // down, not the password wrong.
        SmtpError::Rejected { stage: "AUTH", code: r.code, text: r.text }
    } else {
        SmtpError::Auth(format!("{} {}", r.code, r.text))
    }
}

/// The message as `DATA` wants it: lines ended by CRLF (a bare LF becomes
/// CRLF), a `.` doubled at the start of a line (RFC 5321, 4.5.2), so that
/// no line of the text can end the message early, and a final CRLF.
fn dot_stuff(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + data.len() / 64 + 2);
    let mut at_start = true;
    let mut prev = 0u8;
    for &b in data {
        if at_start && b == b'.' {
            out.push(b'.');
        }
        if b == b'\n' && prev != b'\r' {
            out.push(b'\r');
        }
        out.push(b);
        at_start = b == b'\n';
        prev = b;
    }
    if !out.ends_with(b"\r\n") {
        out.extend_from_slice(b"\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader, DuplexStream};

    /// A scripted server: for each line it receives, the answer of the
    /// first rule whose prefix matches; everything it received, returned.
    async fn server(stream: DuplexStream, ehlo: &'static str, rules: Vec<(&'static str, &'static str)>) -> Vec<String> {
        let (r, mut w) = tokio::io::split(stream);
        let mut r = BufReader::new(r);
        w.write_all(b"220 test ESMTP\r\n").await.unwrap();
        let mut seen = Vec::new();
        let mut in_data = false;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).await.unwrap() == 0 {
                return seen;
            }
            seen.push(line.clone());
            if in_data {
                if line == ".\r\n" {
                    in_data = false;
                    let answer = rules.iter().find(|(p, _)| *p == ".").map(|(_, a)| *a).unwrap_or("250 queued");
                    w.write_all(format!("{answer}\r\n").as_bytes()).await.unwrap();
                }
                continue;
            }
            let answer = if line.starts_with("EHLO") {
                ehlo.to_string()
            } else if let Some((_, a)) = rules.iter().find(|(p, _)| line.starts_with(p)) {
                a.to_string()
            } else if line.starts_with("DATA") {
                in_data = true;
                "354 go ahead".into()
            } else if line.starts_with("QUIT") {
                w.write_all(b"221 bye\r\n").await.unwrap();
                return seen;
            } else {
                "250 ok".into()
            };
            w.write_all(format!("{answer}\r\n").as_bytes()).await.unwrap();
        }
    }

    async fn pair(ehlo: &'static str, rules: Vec<(&'static str, &'static str)>) -> (Result<Client>, tokio::task::JoinHandle<Vec<String>>) {
        let (a, b) = tokio::io::duplex(1 << 16);
        let srv = tokio::spawn(server(b, ehlo, rules));
        (Client::over(a, Timeouts::default(), true).await, srv)
    }

    const EHLO: &str = "250-test\r\n250-SIZE 1000\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME";

    #[tokio::test]
    async fn a_mail_is_sent_with_plain_login() {
        let (c, srv) = pair(EHLO, vec![("AUTH PLAIN", "235 ok")]).await;
        let mut c = c.unwrap();
        assert_eq!(c.extensions().size, Some(1000));
        c.login("me", "secret").await.unwrap();
        let mut ended = false;
        let sent = c.send("me@x.fr", &["a@a.fr".into(), "b@b.fr".into()], b"Subject: x\r\n\r\n.hidden\r\nbody\n", || ended = true).await.unwrap();
        assert!(ended);
        assert!(sent.refused.is_empty());
        c.quit().await;
        let seen = srv.await.unwrap();
        assert!(seen.contains(&format!("AUTH PLAIN {}\r\n", base64(b"\0me\0secret"))), "{seen:?}");
        assert!(seen.contains(&"MAIL FROM:<me@x.fr> SIZE=28\r\n".to_string()), "{seen:?}");
        assert!(seen.contains(&"RCPT TO:<b@b.fr>\r\n".to_string()));
        // The line starting with a dot is doubled, the bare LF fixed.
        assert!(seen.contains(&"..hidden\r\n".to_string()), "{seen:?}");
        assert!(seen.contains(&"body\r\n".to_string()), "{seen:?}");
    }

    #[tokio::test]
    async fn login_falls_back_to_login() {
        let (c, srv) = pair("250-test\r\n250 AUTH LOGIN", vec![("AUTH LOGIN", "334 VXNlcm5hbWU6"), ("bWU=", "334 UGFzc3dvcmQ6"), ("c2VjcmV0", "235 ok")]).await;
        let mut c = c.unwrap();
        c.login("me", "secret").await.unwrap();
        c.quit().await;
        let seen = srv.await.unwrap();
        assert!(seen.contains(&"c2VjcmV0\r\n".to_string()), "{seen:?}");
    }

    #[tokio::test]
    async fn a_refused_login_is_permanent_and_hides_the_password() {
        let (c, _srv) = pair(EHLO, vec![("AUTH", "535 5.7.8 bad credentials")]).await;
        let e = c.unwrap().login("me", "secret").await.unwrap_err();
        assert!(matches!(e, SmtpError::Auth(_)), "{e:?}");
        assert!(e.is_permanent());
        assert!(!e.to_string().contains("secret"));
        let (c, _srv) = pair(EHLO, vec![("AUTH", "454 4.7.0 try later")]).await;
        let e = c.unwrap().login("me", "secret").await.unwrap_err();
        assert!(!e.is_permanent(), "{e:?}");
    }

    #[tokio::test]
    async fn no_password_without_tls() {
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(server(b, EHLO, vec![]));
        let mut c = Client::over(a, Timeouts::default(), false).await.unwrap();
        assert!(matches!(c.login("me", "secret").await, Err(SmtpError::Tls(_))));
    }

    #[tokio::test]
    async fn some_recipients_refused_the_others_get_it() {
        let (c, _srv) = pair(EHLO, vec![("RCPT TO:<bad@", "550 5.1.1 no such user")]).await;
        let mut c = c.unwrap();
        let sent = c.send("me@x.fr", &["ok@a.fr".into(), "bad@a.fr".into()], b"x\r\n", || {}).await.unwrap();
        assert_eq!(sent.refused, vec![Refused { addr: "bad@a.fr".into(), code: 550, text: "5.1.1 no such user".into() }]);
    }

    #[tokio::test]
    async fn every_recipient_refused_sends_nothing() {
        let (c, srv) = pair(EHLO, vec![("RCPT", "550 no")]).await;
        let mut c = c.unwrap();
        let mut ended = false;
        let e = c.send("me@x.fr", &["a@a.fr".into()], b"x\r\n", || ended = true).await.unwrap_err();
        assert!(matches!(e, SmtpError::NoRecipient(ref r) if r.len() == 1), "{e:?}");
        assert!(e.is_permanent());
        assert!(!ended);
        c.quit().await;
        assert!(!srv.await.unwrap().iter().any(|l| l.starts_with("DATA")));
        // A temporary refusal is not permanent.
        let (c, _srv) = pair(EHLO, vec![("RCPT", "450 mailbox busy")]).await;
        let e = c.unwrap().send("me@x.fr", &["a@a.fr".into()], b"x\r\n", || {}).await.unwrap_err();
        assert!(!e.is_permanent());
    }

    #[tokio::test]
    async fn too_large_is_refused_before_sending() {
        let (c, srv) = pair(EHLO, vec![]).await;
        let mut c = c.unwrap();
        let e = c.send("me@x.fr", &["a@a.fr".into()], &[b'x'; 2000], || {}).await.unwrap_err();
        assert_eq!(e, SmtpError::TooLarge { size: 2000, max: 1000 });
        assert!(e.is_permanent());
        c.quit().await;
        assert!(!srv.await.unwrap().iter().any(|l| l.starts_with("MAIL")));
    }

    #[tokio::test]
    async fn answers_to_the_message_itself() {
        let (c, _srv) = pair(EHLO, vec![(".", "554 5.7.1 spam")]).await;
        let e = c.unwrap().send("me@x.fr", &["a@a.fr".into()], b"x\r\n", || {}).await.unwrap_err();
        assert!(matches!(e, SmtpError::Rejected { stage: "DATA", code: 554, .. }), "{e:?}");
        assert!(e.is_permanent());
        let (c, _srv) = pair(EHLO, vec![(".", "451 4.3.0 try again")]).await;
        let e = c.unwrap().send("me@x.fr", &["a@a.fr".into()], b"x\r\n", || {}).await.unwrap_err();
        assert!(!e.is_permanent());
    }

    #[tokio::test]
    async fn hostile_addresses_never_reach_the_server() {
        let (c, srv) = pair(EHLO, vec![]).await;
        let mut c = c.unwrap();
        let e = c.send("me@x.fr", &["a@a.fr>\r\nRCPT TO:<evil@e.fr".into()], b"x\r\n", || {}).await.unwrap_err();
        assert!(matches!(e, SmtpError::InvalidAddress(_)));
        c.quit().await;
        assert!(!srv.await.unwrap().iter().any(|l| l.contains("evil")));
    }

    #[tokio::test]
    async fn a_broken_server_is_a_temporary_failure() {
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let mut b = b;
            b.write_all(b"220 hi\r\n").await.unwrap();
            let mut buf = [0u8; 100];
            let _ = b.read(&mut buf).await;
            b.write_all(b"garbage\r\n").await.unwrap();
        });
        let e = Client::over(a, Timeouts::default(), true).await.err().unwrap();
        assert!(matches!(e, SmtpError::Protocol(_)), "{e:?}");
        assert!(!e.is_permanent());
        // A server that goes away.
        let (a, b) = tokio::io::duplex(1 << 16);
        drop(b);
        let e = Client::over(a, Timeouts::default(), true).await.err().unwrap();
        assert!(matches!(e, SmtpError::Closed | SmtpError::Io(_)), "{e:?}");
        // 421 at the greeting: "not available".
        let (a, b) = tokio::io::duplex(1 << 16);
        tokio::spawn(async move {
            let mut b = b;
            b.write_all(b"421 4.3.2 shutting down\r\n").await.unwrap();
        });
        let e = Client::over(a, Timeouts::default(), true).await.err().unwrap();
        assert!(!e.is_permanent(), "{e:?}");
    }

    #[test]
    fn extensions_are_read_from_ehlo() {
        let lines: Vec<String> = ["mx.example.com", "PIPELINING", "SIZE 35882577", "STARTTLS", "AUTH LOGIN PLAIN XOAUTH2", "SIZE"].iter().map(|s| s.to_string()).collect();
        let e = Extensions::parse(&lines);
        assert!(e.starttls);
        assert_eq!(e.auth, ["LOGIN", "PLAIN", "XOAUTH2"]);
        // The last SIZE has no number: the one before is overwritten by
        // nothing, as the server meant no limit.
        assert_eq!(e.size, None);
        assert_eq!(Extensions::parse(&["x".into(), "size 0".into()]).size, None);
    }

    #[test]
    fn dot_stuffing() {
        assert_eq!(dot_stuff(b".a\r\nb\r\n.\r\n"), b"..a\r\nb\r\n..\r\n");
        assert_eq!(dot_stuff(b"a\nb"), b"a\r\nb\r\n");
        assert_eq!(dot_stuff(b"a.b\r\n"), b"a.b\r\n");
    }
}
