//! What a theme can set (`specifications/themes.md`, section 6): every key,
//! its type, its bounds or choices, and the line of comment that explains
//! it in the files the core writes.
//!
//! The catalog is the single list the rest of the module works from: the
//! reading of a file checks against it, the templates are written from it,
//! the view sent to the app is filled from it, and a test checks that each
//! built-in theme gives every key a value.

/// The type of a key, with its bounds or its choices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// A string among these.
    Choice(&'static [&'static str]),
    Bool,
    /// An integer within the bounds, both included.
    Int(i64, i64),
    /// 0, or an integer within the bounds: a length that 0 turns off
    /// (`layout.reader_max_width`).
    IntOrZero(i64, i64),
    /// A number within the bounds; an integer is accepted (`1` for `1.0`).
    Float(f64, f64),
    /// A color: `"#RRGGBB"`, `"#RRGGBBAA"`, `{ light = …, dark = … }` or
    /// `"system.<name>"` (section 5.1).
    Color,
    /// A font: `"system"`, `"system-mono"`, `"system-serif"`,
    /// `"system-rounded"`, or the name of a family (section 5.2).
    Font,
    /// The blocks of the status bar or the toolbar, in order (section 6.1).
    Blocks,
}

/// One key of a theme.
#[derive(Debug, Clone, Copy)]
pub struct Key {
    /// Its table: `window`, `fonts`, `layout`, `motion`, `colors`, `reader`.
    pub table: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    /// What it does, in the comment of the files the core writes.
    pub role: &'static str,
}

impl Key {
    /// `colors.background`: how problems and the CLI name it.
    pub fn dotted(&self) -> String {
        format!("{}.{}", self.table, self.name)
    }
}

/// The tables of a theme, in the order of the files.
pub const TABLES: &[&str] = &["window", "fonts", "layout", "motion", "colors", "reader"];

/// Blocks of the status bar or the toolbar (section 6.1).
pub const BLOCKS: &[&str] = &["account", "folder", "attributes", "view", "move", "save", "ai", "help"];
/// A wide gap between two groups of blocks.
pub const BLOCK_GAP: &str = "|";
/// All the room left: what follows goes to the right edge.
pub const BLOCK_SPRING: &str = "->";

/// The colors of macOS a theme may name (`"system.accent"`), and the
/// AppKit color each one is (section 5.1). The app translates the names;
/// they follow the appearance, the increased contrast and the accent color
/// chosen in System Settings by themselves.
pub const SYSTEM_COLORS: &[&str] = &[
    "accent",
    "label",
    "secondary_label",
    "tertiary_label",
    "separator",
    "window_background",
    "control_background",
    "under_page_background",
    "text_background",
    "selected_content_background",
    "unemphasized_selected_content_background",
    "alternating_content_background",
    "find_highlight",
    "red",
    "orange",
    "yellow",
    "green",
    "blue",
    "purple",
    "gray",
];

/// The fonts of the system a theme may name instead of a family.
pub const SYSTEM_FONTS: &[&str] = &["system", "system-mono", "system-serif", "system-rounded"];

const fn k(table: &'static str, name: &'static str, kind: Kind, role: &'static str) -> Key {
    Key { table, name, kind, role }
}

/// Every key, in the order of the files.
pub const KEYS: &[Key] = &[
    // [window]: the window and its command (section 6.1).
    k("window", "bar_type", Kind::Choice(&["status_bar", "toolbar"]),
      "command of the window: a compact status bar, or a macOS toolbar whose buttons are the blocks"),
    k("window", "status_bar_position", Kind::Choice(&["top", "bottom", "hidden"]),
      "where the status bar starts; `v` moves it for the window"),
    k("window", "title_bar", Kind::Choice(&["shown", "hidden", "merged"]),
      "merged: the red, yellow and green buttons in the status bar (at the top) or the toolbar"),
    k("window", "key_hints", Kind::Bool, "the blocks remind their key; the keys work either way"),
    k("window", "block_labels", Kind::Bool,
      "the blocks of actions say their name (Display, Move, Save, AI…) beside their icon"),
    k("window", "bar_height", Kind::Int(14, 48), "height of the status bar or toolbar, for a 12 pt text of the bar"),
    k("window", "bar_size", Kind::Choice(&["small", "medium", "large"]),
      "size of the icons and texts of the status bar or toolbar, as in a Mac app: small, medium or large; the bar's height follows"),
    k("window", "block_style", Kind::Choice(&["plain", "pill"]),
      "plain: icon, name and key side by side; pill: each block in a pill, the account and the folder in bold, the block open highlighted"),
    k("window", "bar_line", Kind::Bool, "a line between the status bar and the mails"),
    k("window", "blocks", Kind::Blocks,
      "blocks shown, in order; \"|\" is a wide gap, \"->\" sends what follows to the right; a block left out is hidden, its key still works"),
    k("window", "default_columns", Kind::Int(1, 3), "columns of a first window: 1, 2 or 3"),
    // [fonts] (section 6.2).
    k("fonts", "interface", Kind::Font, "the whole interface: list, bars, menus, pickers"),
    k("fonts", "reading", Kind::Font, "the body of the mails, and the default font of their HTML"),
    k("fonts", "reading_scale", Kind::Float(0.75, 1.5), "size of the body of the mails, times ui.text_size"),
    k("fonts", "reading_line_spacing", Kind::Int(0, 16),
      "points added between two lines of the body of the mails, whatever the text size"),
    k("fonts", "title_scale", Kind::Float(0.75, 2.0), "size of the subject at the top of the reader, times ui.text_size"),
    // [layout]: lengths in points for a 12 pt text (section 5.3).
    k("layout", "row_height", Kind::Int(14, 48), "height of a row of the list"),
    k("layout", "row_radius", Kind::Int(0, 16), "corner radius of the selected row; the buttons of the reader's title line are rounded in proportion"),
    k("layout", "row_inset", Kind::Int(0, 16), "margin of the selected row from the edges of the list"),
    k("layout", "zebra", Kind::Bool, "alternate row backgrounds"),
    k("layout", "menu_row_height", Kind::IntOrZero(14, 40),
      "height of a row of the menus and pickers; 0: the height of its text"),
    k("layout", "popup_radius", Kind::Int(0, 24), "corner radius of menus, pickers, questions and panels"),
    k("layout", "popup_shadow", Kind::Bool, "a shadow under them"),
    k("layout", "popup_padding", Kind::Int(4, 32),
      "margin inside menus, pickers and panels, around their title and rows"),
    k("layout", "menu_keys", Kind::Choice(&["left", "right"]), "side of the key on a menu row"),
    k("layout", "message_cards", Kind::Bool, "each message of the conversation in a rounded frame"),
    k("layout", "avatars", Kind::Bool, "the initials of the sender before each message (nothing is downloaded)"),
    k("layout", "reader_max_width", Kind::IntOrZero(320, 1200),
      "widest text of the reader, centered; 0: the whole width"),
    k("layout", "reader_title_padding", Kind::Int(0, 32),
      "room above and below the subject of the reader, also added to the 12 pt on its sides; half of it between the two lines of a long title"),
    k("layout", "scrollers", Kind::Choice(&["none", "thin", "standard"]), "scroll bars"),
    k("layout", "separators", Kind::Choice(&["lines", "space"]), "areas set apart by lines, or mostly by white space"),
    // [motion]: milliseconds, 0 when System Settings reduce motion (6.4).
    k("motion", "sidebar_ms", Kind::Int(0, 600), "the folder column slides in"),
    k("motion", "popup_ms", Kind::Int(0, 300), "menus and pickers fade in"),
    // [colors] (section 6.5).
    k("colors", "background", Kind::Color, "window and reader"),
    k("colors", "sidebar", Kind::Color, "folder column"),
    k("colors", "list", Kind::Color, "list of the mails"),
    k("colors", "bar", Kind::Color, "status bar or toolbar"),
    k("colors", "text", Kind::Color, "text"),
    k("colors", "muted", Kind::Color, "secondary text: dates, counts, headers"),
    k("colors", "accent", Kind::Color, "what is not text: a frame"),
    k("colors", "accent_text", Kind::Color, "keys of the menus, tags, caret"),
    k("colors", "selection", Kind::Color, "selected row"),
    k("colors", "selection_text", Kind::Color, "text of the selected row"),
    k("colors", "marked", Kind::Color, "rows marked for a grouped action"),
    k("colors", "marked_bar", Kind::Color, "bar of the marked rows"),
    k("colors", "separator", Kind::Color, "lines"),
    k("colors", "popup", Kind::Color, "menus, pickers, panels"),
    k("colors", "unread", Kind::Color, "dot of the unread mails"),
    k("colors", "flag", Kind::Color, "flag"),
    k("colors", "error", Kind::Color, "errors"),
    k("colors", "virtual_folder", Kind::Color, "virtual folders: Work, All Mail, Everything"),
    // [reader] (section 6.6): how a received mail is shown. Only the
    // reading: what is sent stays `compose.format` of `config.toml`, so
    // that a theme never changes the MIME the recipients get.
    k("reader", "body_format", Kind::Choice(&["text", "html"]),
      "mails shown as text or HTML when nothing is remembered for their sender; vh remembers a choice for a sender, which wins"),
];

/// The key `name` of `table`.
pub fn key(table: &str, name: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.table == table && k.name == name)
}

/// The pairs whose contrast is checked (section 6.5): a text color and the
/// background it is read on.
pub const CONTRAST_PAIRS: &[(&str, &str)] = &[
    ("text", "background"),
    ("text", "list"),
    ("muted", "background"),
    ("muted", "list"),
    ("accent_text", "background"),
    ("accent_text", "list"),
    ("selection_text", "selection"),
];

/// The least contrast between a text and its background, WCAG AA for
/// normal text.
pub const MIN_CONTRAST: f64 = 4.5;

/// The themes shipped with the app, embedded in the core. The first one is
/// the default, and the parent of a theme that does not say its own.
pub const BUILTINS: &[&str] = &["geek", "clean", "regular"];

/// The default theme.
pub const DEFAULT_THEME: &str = "geek";

/// Whether `name` can name a theme file: lowercase letters, digits and
/// hyphens, starting with a letter or a digit, 40 at most.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
}
