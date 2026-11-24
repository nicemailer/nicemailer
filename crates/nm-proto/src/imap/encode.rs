//! Writing commands: the arguments, with the quoting rules of RFC 3501.
//!
//! Commands are written here rather than with the encoder of `imap-codec`:
//! some that NiceMailer needs cannot be expressed with its types
//! (`LIST … RETURN (SPECIAL-USE)`, `SELECT … (QRESYNC …)`), and writing them
//! is the easy half of IMAP. The hard half, reading the responses, stays
//! with `imap-codec`. What must be right here is the quoting of the values
//! that come from outside (user names, mailbox names, search values): each
//! goes through [`Command::astring`] or [`Command::string`], which choose an
//! atom, a quoted string or a literal, so that no value can end the line
//! early or inject a second command.

/// A command without its tag, as pieces: text, and literals, which the
/// session sends after the server's go-ahead (`+`) unless `LITERAL+` lets
/// it send them at once (`session.rs`, `send`).
#[derive(Debug, Clone)]
pub(crate) struct Command {
    /// Name of the command, for error messages (`UID FETCH`).
    pub name: &'static str,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Part {
    Text(Vec<u8>),
    /// Bytes sent as `{n}` then the bytes.
    Literal(Vec<u8>),
    /// End of a line: wait for the server's `+` before the rest
    /// (`AUTHENTICATE` without `SASL-IR`, whose response follows on a line
    /// of its own).
    Wait,
}

impl Command {
    /// A command starting with its name (`UID FETCH`).
    pub fn new(name: &'static str) -> Command {
        Command { name, parts: vec![Part::Text(name.as_bytes().to_vec())] }
    }

    /// Appends text as it is, after a space. Only for text written by
    /// NiceMailer itself (keywords, numbers, sets of UIDs), never for a
    /// value that comes from outside.
    pub fn raw(mut self, text: &str) -> Command {
        self.text(b" ");
        self.text(text.as_bytes());
        self
    }

    /// Appends text without a space, for the inside of a parenthesized list.
    pub fn raw_tight(mut self, text: &str) -> Command {
        self.text(text.as_bytes());
        self
    }

    /// Appends an `astring` (RFC 3501, section 9): an atom when the value
    /// allows it, else a quoted string, else a literal.
    pub fn astring(mut self, value: &[u8]) -> Command {
        self.text(b" ");
        if !value.is_empty() && value.iter().all(|&b| is_astring_char(b)) {
            self.text(value);
        } else {
            self.push_string(value);
        }
        self
    }

    /// Appends a `string`: quoted when the value allows it, else a literal.
    /// For the places where an atom would be ambiguous (the values of `ID`,
    /// where `NIL` means "no value").
    pub fn string(mut self, value: &[u8]) -> Command {
        self.text(b" ");
        self.push_string(value);
        self
    }

    /// Appends a literal, whatever the value: for message contents
    /// (`APPEND`).
    pub fn literal(mut self, value: Vec<u8>) -> Command {
        self.text(b" ");
        self.parts.push(Part::Literal(value));
        self
    }

    /// Ends the line here and waits for the server's `+`.
    pub fn wait(mut self) -> Command {
        self.parts.push(Part::Wait);
        self
    }

    fn push_string(&mut self, value: &[u8]) {
        if value.len() <= MAX_QUOTED && value.iter().all(|&b| is_quoted_char(b)) {
            let mut q = Vec::with_capacity(value.len() + 2);
            q.push(b'"');
            for &b in value {
                if b == b'"' || b == b'\\' {
                    q.push(b'\\');
                }
                q.push(b);
            }
            q.push(b'"');
            self.text(&q);
        } else {
            self.parts.push(Part::Literal(value.to_vec()));
        }
    }

    /// Appends to the last text piece, or starts one after a literal.
    fn text(&mut self, bytes: &[u8]) {
        match self.parts.last_mut() {
            Some(Part::Text(t)) => t.extend_from_slice(bytes),
            _ => self.parts.push(Part::Text(bytes.to_vec())),
        }
    }

    /// The command as one block of bytes, literals written as
    /// synchronizing (`{n}`). For tests, and for the length check.
    #[cfg(test)]
    pub fn flatten(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for p in &self.parts {
            match p {
                Part::Text(t) => out.extend_from_slice(t),
                Part::Literal(l) => {
                    out.extend_from_slice(format!("{{{}}}\r\n", l.len()).as_bytes());
                    out.extend_from_slice(l);
                }
                Part::Wait => out.extend_from_slice(b"\r\n"),
            }
        }
        out
    }
}

/// Longest value sent as a quoted string; longer ones go as literals,
/// which servers accept at any size.
const MAX_QUOTED: usize = 1024;

/// `ASTRING-CHAR`: `ATOM-CHAR` or `]`. An atom char is a printable ASCII
/// character other than the specials `( ) { SP % * " \ ]`.
fn is_astring_char(b: u8) -> bool {
    (0x21..=0x7e).contains(&b) && !matches!(b, b'(' | b')' | b'{' | b'%' | b'*' | b'"' | b'\\')
}

/// `TEXT-CHAR` of a quoted string: 7-bit, no NUL, no CR or LF.
fn is_quoted_char(b: u8) -> bool {
    (0x01..=0x7f).contains(&b) && b != b'\r' && b != b'\n'
}

/// The initial response of SASL `XOAUTH2` (the mechanism Microsoft
/// documents for IMAP, POP and SMTP), base64-encoded:
/// `user={user}\x01auth=Bearer {token}\x01\x01`
/// (`specifications/oauth2.md`, section 7). The token is a bearer token:
/// whoever holds it reads the mailbox, so it travels only inside the
/// base64 of a TLS connection, and never into an error or a log.
pub(crate) fn xoauth2(user: &str, token: &str) -> String {
    let mut raw = Vec::with_capacity(user.len() + token.len() + 22);
    raw.extend_from_slice(b"user=");
    raw.extend_from_slice(user.as_bytes());
    raw.push(1);
    raw.extend_from_slice(b"auth=Bearer ");
    raw.extend_from_slice(token.as_bytes());
    raw.extend_from_slice(&[1, 1]);
    base64(&raw)
}

/// Standard base64 (RFC 4648), with padding: the response of
/// `AUTHENTICATE PLAIN`.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const STD: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(STD[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(c: &Command) -> String {
        String::from_utf8(c.flatten()).unwrap()
    }

    #[test]
    fn xoauth2_is_the_documented_initial_response() {
        // `user=alice\x01auth=Bearer tok\x01\x01`, base64 (RFC 4648).
        assert_eq!(xoauth2("alice", "tok"), "dXNlcj1hbGljZQFhdXRoPUJlYXJlciB0b2sBAQ==");
    }

    #[test]
    fn astring_picks_atom_quoted_or_literal() {
        assert_eq!(text(&Command::new("LOGIN").astring(b"alice")), "LOGIN alice");
        assert_eq!(text(&Command::new("LOGIN").astring(b"alice@example.com")), "LOGIN alice@example.com");
        assert_eq!(text(&Command::new("X").astring(b"a]b")), "X a]b", "] is allowed in an astring");
        assert_eq!(text(&Command::new("X").astring(b"")), "X \"\"");
        assert_eq!(text(&Command::new("X").astring(b"two words")), "X \"two words\"");
        assert_eq!(text(&Command::new("X").astring(b"a\"b\\c")), "X \"a\\\"b\\\\c\"");
        assert_eq!(text(&Command::new("X").astring(b"a*b")), "X \"a*b\"");
        assert_eq!(text(&Command::new("X").astring(b"{5}")), "X \"{5}\"");
        assert_eq!(text(&Command::new("X").astring("é".as_bytes())), "X {2}\r\né");
    }

    #[test]
    fn no_value_can_end_the_line() {
        // A line break in a value would end the command and start another:
        // it must go in a literal, whose length the server reads first.
        let c = Command::new("LOGIN").astring(b"alice\r\nA2 DELETE INBOX");
        assert_eq!(c.parts.len(), 2);
        assert_eq!(c.parts[1], Part::Literal(b"alice\r\nA2 DELETE INBOX".to_vec()));
        let c = Command::new("X").string(&[b'a', 0, b'b']);
        assert!(matches!(c.parts[1], Part::Literal(_)), "NUL is not allowed in a quoted string");
        let long = vec![b'a'; MAX_QUOTED + 1];
        assert!(matches!(Command::new("X").string(&long).parts[1], Part::Literal(_)));
    }

    #[test]
    fn text_after_a_literal_starts_a_new_piece() {
        let c = Command::new("APPEND").astring(b"INBOX").literal(b"abc".to_vec()).raw("tail");
        assert_eq!(c.parts, vec![
            Part::Text(b"APPEND INBOX ".to_vec()),
            Part::Literal(b"abc".to_vec()),
            Part::Text(b" tail".to_vec()),
        ]);
    }

    #[test]
    fn string_quotes_even_atoms() {
        assert_eq!(text(&Command::new("ID").raw("(").raw_tight("").string(b"name")), "ID ( \"name\"");
        assert_eq!(text(&Command::new("X").string(b"NIL")), "X \"NIL\"");
    }

    #[test]
    fn base64_of_rfc_4648() {
        for (input, out) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), out);
        }
        assert_eq!(base64(b"\0alice\0secret"), "AGFsaWNlAHNlY3JldA==");
    }
}
