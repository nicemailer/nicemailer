//! How a key is written in `keymap.toml` (`raccourcis.md`, section 4):
//! `j`, `J`, `?`, `ctrl-d`, `cmd-shift-a`, `shift-space`, `enter`.
//!
//! Two kinds of keys, read differently, because macOS reports them
//! differently:
//!
//! - **without ⌃, ⌥ nor ⌘**, a key is the *character it types*: `?` is the
//!   key that types `?`, wherever the keyboard layout puts it, and `J`
//!   means Shift held on `j`. This is what the app has always compared
//!   (`event.characters`), and it is what lets AZERTY users press `?`
//!   without thinking of where it sits on a QWERTY keyboard;
//! - **with ⌃, ⌥ or ⌘**, a key is the letter *printed on the key*
//!   (`charactersIgnoringModifiers`), because ⌥ types other characters
//!   (⌥n is `~` on AZERTY) and ⌃ control characters.
//!
//! In both cases Shift is folded into the character for the keys that are
//! not letters: Shift on `/` types `?` in QWERTY but `.` in AZERTY, so
//! `shift-/` would mean different keys on different Macs. It is refused,
//! and `?` is written instead. For letters, Shift is kept as a flag, and
//! written either way: `J` and `shift-j` are the same key, as are
//! `cmd-A` and `cmd-shift-a`.
//!
//! [`Chord::canonical`] gives the one spelling of a key, used to find two
//! spellings of the same key in a table, and in what the core sends the
//! app (`keymap_get`), so that the app compares strings it builds the same
//! way from an `NSEvent`.

use std::fmt;

/// A key with its modifiers: one line of a table of `keymap.toml`.
///
/// Rust note: `#[derive(...)]` asks the compiler to write the code of these
/// traits: `Clone`/`Copy` (the value is copied like an integer, it owns no
/// memory), `PartialEq`/`Eq`/`Hash` (it can be compared and be a map key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Chord {
    pub ctrl: bool,
    pub cmd: bool,
    pub alt: bool,
    /// Only for letters and named keys: for the other characters, Shift is
    /// part of the character (see the module's comment).
    pub shift: bool,
    pub key: Key,
}

/// The key itself, without its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    /// A character; a letter is kept in lower case, its Shift in
    /// [`Chord::shift`].
    Char(char),
    Named(Named),
}

/// The keys that type no character, or whose character is invisible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Named {
    Space,
    Enter,
    Tab,
    Esc,
    Backspace,
    Del,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    /// `f1` to `f12`.
    F(u8),
}

/// The names of [`Named`], as written in the file. Rust note: a `const`
/// array of tuples is built at compile time and costs nothing at run time.
const NAMES: [(&str, Named); 14] = [
    ("space", Named::Space),
    ("enter", Named::Enter),
    ("tab", Named::Tab),
    ("esc", Named::Esc),
    ("backspace", Named::Backspace),
    ("del", Named::Del),
    ("up", Named::Up),
    ("down", Named::Down),
    ("left", Named::Left),
    ("right", Named::Right),
    ("home", Named::Home),
    ("end", Named::End),
    ("pageup", Named::PageUp),
    ("pagedown", Named::PageDown),
];

impl Named {
    fn name(self) -> String {
        match self {
            Named::F(n) => format!("f{n}"),
            other => NAMES.iter().find(|(_, k)| *k == other).map(|(n, _)| n.to_string()).unwrap_or_default(),
        }
    }

    fn from_name(s: &str) -> Option<Named> {
        if let Some((_, k)) = NAMES.iter().find(|(n, _)| *n == s) {
            return Some(*k);
        }
        // `f1` … `f12`. Rust note: `strip_prefix` returns the rest of the
        // string after the prefix, or `None` when it does not start with it.
        let n: u8 = s.strip_prefix('f')?.parse().ok()?;
        (1..=12).contains(&n).then_some(Named::F(n))
    }
}

/// Whether `c` is a letter with an upper and a lower case: the characters
/// whose Shift is kept as a flag. A digit, a symbol, or a letter without
/// case (CJK) has its Shift folded into the character.
fn has_case(c: char) -> bool {
    c.to_lowercase().ne(c.to_uppercase())
}

/// `c` in lower case, when that is one character (it always is for the
/// letters of a keyboard; `İ` is the exception that gives two).
fn lower(c: char) -> Option<char> {
    let mut l = c.to_lowercase();
    // Rust note: `let (Some(x), None) = …` matches "exactly one item".
    match (l.next(), l.next()) {
        (Some(x), None) => Some(x),
        _ => None,
    }
}

impl Chord {
    /// Reads one key of the file. The error says what is wrong, in words
    /// for the user (the caller adds the line).
    pub fn parse(text: &str) -> Result<Chord, String> {
        let mut rest = text;
        let (mut ctrl, mut cmd, mut alt, mut shift) = (false, false, false, false);
        // The modifiers, in any order. A string of a single character is a
        // key, even `-`: `cmd--` is ⌘ and the key `-`.
        loop {
            if rest.chars().count() <= 1 {
                break;
            }
            let (flag, tail) = if let Some(t) = rest.strip_prefix("ctrl-") {
                (&mut ctrl, t)
            } else if let Some(t) = rest.strip_prefix("cmd-") {
                (&mut cmd, t)
            } else if let Some(t) = rest.strip_prefix("alt-") {
                (&mut alt, t)
            } else if let Some(t) = rest.strip_prefix("shift-") {
                (&mut shift, t)
            } else {
                break;
            };
            if *flag {
                return Err(format!("{text:?}: a modifier is written twice"));
            }
            *flag = true;
            rest = tail;
        }
        if rest.is_empty() {
            return Err(format!("{text:?}: a modifier needs a key after it"));
        }

        let mut chars = rest.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(' '), None) => Key::Named(Named::Space),
            (Some(c), None) => {
                if c.is_control() || c.is_whitespace() {
                    return Err(format!("{text:?}: an invisible character; write its name (space, tab, enter…)"));
                }
                if has_case(c) {
                    let l = lower(c).ok_or_else(|| format!("{text:?}: this letter has no single lower case"))?;
                    // Rust note: `|=` is "or-assign": an upper case letter
                    // adds Shift, written or not.
                    shift |= l != c;
                    Key::Char(l)
                } else {
                    if shift {
                        return Err(format!(
                            "{text:?}: Shift is part of the character for a key that is not a letter; write the \
                             character Shift types (`?` rather than `shift-/`)"
                        ));
                    }
                    Key::Char(c)
                }
            }
            _ => match Named::from_name(rest) {
                Some(n) => Key::Named(n),
                None => {
                    return Err(format!(
                        "{text:?}: unknown key {rest:?}; a key is one character, or a name among space, enter, tab, \
                         esc, backspace, del, up, down, left, right, home, end, pageup, pagedown, f1…f12, after \
                         the modifiers ctrl-, cmd-, alt-, shift-"
                    ));
                }
            },
        };
        Ok(Chord { ctrl, cmd, alt, shift, key })
    }

    /// The one spelling of the key: modifiers in the order `ctrl-cmd-alt-
    /// shift-`, a letter with Shift and no other modifier in upper case
    /// (`J`, not `shift-j`), otherwise in lower case with `shift-`.
    pub fn canonical(&self) -> String {
        let mut s = String::new();
        let bare = !self.ctrl && !self.cmd && !self.alt;
        if self.ctrl {
            s.push_str("ctrl-");
        }
        if self.cmd {
            s.push_str("cmd-");
        }
        if self.alt {
            s.push_str("alt-");
        }
        match self.key {
            Key::Char(c) if bare && self.shift => s.extend(c.to_uppercase()),
            Key::Char(c) => {
                if self.shift {
                    s.push_str("shift-");
                }
                s.push(c);
            }
            Key::Named(n) => {
                if self.shift {
                    s.push_str("shift-");
                }
                s.push_str(&n.name());
            }
        }
        s
    }

    /// Whether the key types a character: one without ⌃ nor ⌘ (⌥ types
    /// characters too on a Mac). Refused where text is typed, where it
    /// would make the character impossible to type (`raccourcis.md`,
    /// section 5.2).
    pub fn types_character(&self) -> bool {
        !self.ctrl && !self.cmd && matches!(self.key, Key::Char(_) | Key::Named(Named::Space))
    }

    /// Whether the key, without ⌃ ⌥ ⌘ (Shift allowed), edits or moves in a
    /// text: what the editor of a reply needs for itself (`raccourcis.md`,
    /// section 5.2). The function keys and Escape are not.
    pub fn edits_text(&self) -> bool {
        !self.ctrl
            && !self.cmd
            && !self.alt
            && matches!(
                self.key,
                Key::Named(
                    Named::Enter
                        | Named::Tab
                        | Named::Backspace
                        | Named::Del
                        | Named::Up
                        | Named::Down
                        | Named::Left
                        | Named::Right
                        | Named::Home
                        | Named::End
                        | Named::PageUp
                        | Named::PageDown
                )
            )
    }

    /// Whether the key is one of the fixed keys of a line being typed: ←,
    /// →, ⌫ or ⌦, alone or with Shift (`raccourcis.md`, section 5.3).
    pub fn edits_line(&self) -> bool {
        !self.ctrl
            && !self.cmd
            && !self.alt
            && matches!(self.key, Key::Named(Named::Left | Named::Right | Named::Backspace | Named::Del))
    }

    /// The keys no file may bind nor remove (`raccourcis.md`, section 4.4):
    /// Escape alone, which must always go back, and ⌘Q.
    pub fn is_reserved(&self) -> bool {
        let only = |cmd: bool| !self.ctrl && !self.alt && !self.shift && self.cmd == cmd;
        (self.key == Key::Named(Named::Esc) && only(false)) || (self.key == Key::Char('q') && only(true))
    }
}

// Rust note: implementing `Display` gives `to_string()` and `{}` in
// `format!`, here the canonical spelling.
impl fmt::Display for Chord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.canonical())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(s: &str) -> String {
        Chord::parse(s).unwrap_or_else(|e| panic!("{s}: {e}")).canonical()
    }

    #[test]
    fn characters_are_read_as_typed() {
        for s in ["j", "?", "*", "@", ";", "/", "-", "1", "é", "#"] {
            assert_eq!(canon(s), s);
        }
    }

    #[test]
    fn shift_on_a_letter_is_written_either_way() {
        assert_eq!(canon("J"), "J");
        assert_eq!(canon("shift-j"), "J");
        assert_eq!(canon("shift-J"), "J");
        assert_eq!(Chord::parse("J").unwrap(), Chord::parse("shift-j").unwrap());
        assert_eq!(canon("ctrl-C"), "ctrl-shift-c");
        assert_eq!(canon("cmd-A"), "cmd-shift-a");
        assert_eq!(canon("shift-cmd-a"), "cmd-shift-a");
    }

    #[test]
    fn modifiers_in_any_order_give_one_spelling() {
        assert_eq!(canon("cmd-ctrl-f"), "ctrl-cmd-f");
        assert_eq!(canon("shift-alt-cmd-v"), "cmd-alt-shift-v");
        assert_eq!(canon("cmd-shift-enter"), "cmd-shift-enter");
        assert_eq!(canon("shift-cmd-enter"), "cmd-shift-enter");
    }

    #[test]
    fn a_minus_key_after_a_modifier() {
        assert_eq!(canon("cmd--"), "cmd--");
        assert_eq!(canon("cmd-+"), "cmd-+");
        assert_eq!(canon("-"), "-");
    }

    #[test]
    fn named_keys() {
        for s in ["space", "enter", "tab", "esc", "backspace", "del", "up", "down", "left", "right", "home", "end",
            "pageup", "pagedown", "f1", "f12", "shift-space", "shift-tab", "cmd-esc", "alt-backspace"]
        {
            assert_eq!(canon(s), s);
        }
        assert_eq!(canon(" "), "space");
    }

    #[test]
    fn invalid_keys_are_refused_with_a_reason() {
        for (s, says) in [
            ("", "needs a key"),
            ("ctrl-", "needs a key"),
            ("shift-/", "Shift is part of the character"),
            ("shift-1", "Shift is part of the character"),
            ("ctrl-ctrl-a", "twice"),
            ("Enter", "unknown key"),
            ("return", "unknown key"),
            ("f13", "unknown key"),
            ("option-a", "unknown key"),
            ("ab", "unknown key"),
            ("\t", "invisible"),
        ] {
            let e = Chord::parse(s).expect_err(s);
            assert!(e.contains(says), "{s:?}: {e}");
        }
    }

    #[test]
    fn canonical_spelling_reads_back_as_the_same_key() {
        for s in ["J", "ctrl-shift-c", "cmd-alt-shift-v", "shift-down", "?", "cmd--", "f5", "alt-é"] {
            let k = Chord::parse(s).unwrap();
            assert_eq!(Chord::parse(&k.canonical()).unwrap(), k, "{s}");
        }
    }

    #[test]
    fn which_keys_type_a_character() {
        assert!(Chord::parse("j").unwrap().types_character());
        assert!(Chord::parse("J").unwrap().types_character());
        assert!(Chord::parse("alt-n").unwrap().types_character());
        assert!(!Chord::parse("ctrl-a").unwrap().types_character());
        assert!(!Chord::parse("cmd-v").unwrap().types_character());
        assert!(!Chord::parse("enter").unwrap().types_character());
        assert!(!Chord::parse("alt-left").unwrap().types_character());
        assert!(Chord::parse("space").unwrap().types_character());
        assert!(Chord::parse("shift-space").unwrap().types_character());
        assert!(!Chord::parse("ctrl-space").unwrap().types_character());
    }

    #[test]
    fn which_keys_edit_a_text() {
        for s in ["enter", "shift-enter", "tab", "backspace", "del", "up", "shift-down", "home", "pagedown"] {
            assert!(Chord::parse(s).unwrap().edits_text(), "{s}");
        }
        for s in ["cmd-enter", "alt-left", "f5", "esc", "ctrl-a"] {
            assert!(!Chord::parse(s).unwrap().edits_text(), "{s}");
        }
        assert!(Chord::parse("shift-left").unwrap().edits_line());
        assert!(!Chord::parse("home").unwrap().edits_line());
        assert!(!Chord::parse("alt-backspace").unwrap().edits_line());
    }

    #[test]
    fn reserved_keys() {
        assert!(Chord::parse("esc").unwrap().is_reserved());
        assert!(Chord::parse("cmd-q").unwrap().is_reserved());
        assert!(!Chord::parse("cmd-esc").unwrap().is_reserved());
        assert!(!Chord::parse("shift-esc").unwrap().is_reserved());
        assert!(!Chord::parse("cmd-shift-q").unwrap().is_reserved());
        assert!(!Chord::parse("q").unwrap().is_reserved());
    }
}
