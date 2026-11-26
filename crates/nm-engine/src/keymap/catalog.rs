//! The contexts of the keys and the actions they can be bound to
//! (`raccourcis.md`, sections 5 and 6).
//!
//! The core does not run these actions: the app does. It knows them to
//! validate a file, so that `nicemailer keymap check` and the banner of
//! the app say "this action does not exist" or "this action makes no sense
//! here" with the line, instead of the app ignoring a binding silently.
//! The app has the same list (`KeyAction`, in Swift); a test of the app
//! checks the two agree, through the JSON of the defaults
//! (`keymap::defaults_json`).

/// Where a key is looked for. A key is looked for in the most precise
/// context active, then in its parents, up to [`Context::Everywhere`]
/// (`raccourcis.md`, section 5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Context {
    Everywhere,
    Normal,
    Mail,
    Quarantine,
    Screen,
    Contacts,
    Tags,
    Groups,
    History,
    MenuNavigation,
    Picker,
    ContactFilter,
    LineEdit,
    Choice,
    AiPanel,
    Compose,
}

impl Context {
    /// Every context, in the order of the file of defaults.
    pub const ALL: [Context; 16] = [
        Context::Everywhere,
        Context::Normal,
        Context::Mail,
        Context::Quarantine,
        Context::Screen,
        Context::Contacts,
        Context::Tags,
        Context::Groups,
        Context::History,
        Context::MenuNavigation,
        Context::Picker,
        Context::ContactFilter,
        Context::LineEdit,
        Context::Choice,
        Context::AiPanel,
        Context::Compose,
    ];

    /// The name of its table in `keymap.toml`.
    pub fn name(self) -> &'static str {
        match self {
            Context::Everywhere => "everywhere",
            Context::Normal => "normal",
            Context::Mail => "mail",
            Context::Quarantine => "quarantine",
            Context::Screen => "screen",
            Context::Contacts => "contacts",
            Context::Tags => "tags",
            Context::Groups => "groups",
            Context::History => "history",
            Context::MenuNavigation => "menu_navigation",
            Context::Picker => "picker",
            Context::ContactFilter => "contact_filter",
            Context::LineEdit => "line_edit",
            Context::Choice => "choice",
            Context::AiPanel => "ai_panel",
            Context::Compose => "compose",
        }
    }

    pub fn from_name(name: &str) -> Option<Context> {
        Context::ALL.into_iter().find(|c| c.name() == name)
    }

    /// The context looked into when a key is not found in this one.
    pub fn parent(self) -> Option<Context> {
        use Context::*;
        match self {
            Everywhere => None,
            Normal | MenuNavigation | LineEdit | Choice | AiPanel | Compose => Some(Everywhere),
            Mail | Screen => Some(Normal),
            Quarantine => Some(Mail),
            Contacts | Tags | Groups | History => Some(Screen),
            Picker | ContactFilter => Some(LineEdit),
        }
    }

    /// Whether `self` is `ancestor` or one of its descendants: a binding of
    /// `ancestor` applies in `self`.
    pub fn within(self, ancestor: Context) -> bool {
        // Rust note: `std::iter::successors` builds the chain self,
        // parent, grandparent… until `parent` gives `None`.
        std::iter::successors(Some(self), |c| c.parent()).any(|c| c == ancestor)
    }

    /// Where text is typed: a key typing a character cannot be bound there
    /// (`raccourcis.md`, section 5.2). `everywhere` is one, since its keys
    /// apply while a reply is written.
    pub fn is_typing(self) -> bool {
        matches!(self, Context::Everywhere | Context::Compose | Context::LineEdit | Context::Picker | Context::ContactFilter)
    }
}

/// An action of the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Action {
    /// Its name in the file, e.g. `mail.archive`.
    pub id: &'static str,
    /// The highest context it can be bound in: it can be bound there and
    /// in every descendant (`move.inbox`, of `mail`, also in `quarantine`),
    /// nowhere else (`compose.send` makes no sense in `contacts`).
    pub scope: Context,
}

impl Action {
    /// Whether it may be a row of a menu (`raccourcis.md`, section 7): the
    /// commands of the mail screen and of every screen, not the keys that
    /// move in a panel, and not another menu (menus do not nest).
    pub fn in_menu(&self) -> bool {
        matches!(self.scope, Context::Everywhere | Context::Normal | Context::Mail) && !self.opens_menu()
    }

    /// The action opens a menu: `menu.<name>`, except the moves inside one.
    pub fn opens_menu(&self) -> bool {
        menu_name(self.id).is_some()
    }
}

/// The name of the menu `id` opens (`menu.save` → `save`), or `None` for
/// another action. `menu.next`, `menu.previous`, `menu.choose` and
/// `menu.choose_alt` move inside an open menu: they open nothing.
pub fn menu_name(id: &str) -> Option<&str> {
    let name = id.strip_prefix("menu.")?;
    (!NAVIGATION.contains(&name)).then_some(name)
}

/// The actions of `menu_navigation`, which share the `menu.` prefix with
/// the openers of the menus.
const NAVIGATION: [&str; 4] = ["next", "previous", "choose", "choose_alt"];

/// The menus the app provides (`raccourcis.md`, section 7). A table
/// `[menu.<name>]` with another name creates a menu of the user.
pub const BUILTIN_MENUS: [&str; 4] = ["move", "goto", "save", "view"];

/// Whether `name` can name a menu of the user: `[a-z0-9_]`, 1 to 32
/// characters, so it reads as a word in the status bar and in the help.
pub fn valid_menu_name(name: &str) -> bool {
    (1..=32).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Shorthand for the table below.
const fn a(id: &'static str, scope: Context) -> Action {
    Action { id, scope }
}

use Context::{AiPanel, Choice, Compose, ContactFilter, Contacts, Everywhere, History, LineEdit, Mail, MenuNavigation, Normal, Picker, Screen};

/// Every action, grouped as in the file of defaults. Each has at least one
/// key by default (a test checks it): an action without a key would not
/// appear in the help, and nobody would learn it exists.
pub const ACTIONS: &[Action] = &[
    // everywhere
    a("window.full_screen", Everywhere),
    a("text.bigger", Everywhere),
    a("text.smaller", Everywhere),
    a("text.reset", Everywhere),
    a("app.reload", Everywhere),
    // normal: every screen
    a("menu.goto", Normal),
    a("help.show", Normal),
    a("undo.last", Normal),
    a("goto.inbox", Normal),
    a("goto.todo", Normal),
    a("goto.waiting", Normal),
    a("goto.work", Normal),
    a("goto.archive", Normal),
    a("goto.newsletter", Normal),
    a("goto.paperwork", Normal),
    a("goto.sent", Normal),
    a("goto.drafts", Normal),
    a("goto.all_mail", Normal),
    a("goto.quarantine", Normal),
    a("goto.deleted", Normal),
    a("goto.junk", Normal),
    a("goto.banned", Normal),
    a("goto.everything", Normal),
    a("goto.contacts", Normal),
    a("goto.tags", Normal),
    a("goto.groups", Normal),
    a("goto.history", Normal),
    // mail
    a("list.down", Mail),
    a("list.up", Mail),
    a("list.mark_down", Mail),
    a("list.mark_up", Mail),
    a("list.mark_all", Mail),
    a("list.half_page_down", Mail),
    a("list.half_page_up", Mail),
    a("message.next", Mail),
    a("message.previous", Mail),
    a("mail.open", Mail),
    a("reader.open_or_page_down", Mail),
    a("reader.page_down", Mail),
    a("reader.page_up", Mail),
    a("reader.toggle_all", Mail),
    a("reader.skip_quoted", Mail),
    a("picker.links", Mail),
    a("sender.card_or_copy", Mail),
    a("sender.mails", Mail),
    a("mail.archive", Mail),
    a("menu.move", Mail),
    a("mail.delete", Mail),
    a("mail.toggle_seen", Mail),
    a("mail.mark_seen", Mail),
    a("mail.toggle_flag", Mail),
    a("picker.tag", Mail),
    a("picker.group", Mail),
    a("menu.save", Mail),
    a("mail.unsubscribe", Mail),
    a("compose.reply_all", Mail),
    a("compose.reply_sender", Mail),
    a("compose.forward", Mail),
    a("compose.new", Mail),
    a("ai.menu", Mail),
    a("picker.search", Mail),
    a("picker.attributes", Mail),
    a("picker.accounts", Mail),
    a("menu.view", Mail),
    a("move.inbox", Mail),
    a("move.todo", Mail),
    a("move.waiting", Mail),
    a("move.archive", Mail),
    a("move.newsletter", Mail),
    a("move.paperwork", Mail),
    a("move.quarantine", Mail),
    a("move.deleted", Mail),
    a("move.junk", Mail),
    a("move.banned", Mail),
    a("save.clipboard", Mail),
    a("save.eml", Mail),
    a("save.text", Mail),
    a("save.text_open", Mail),
    a("save.markdown", Mail),
    a("save.markdown_open", Mail),
    a("save.attachments", Mail),
    a("save.attachments_no_graphics", Mail),
    a("save.pdf", Mail),
    a("save.pdf_open", Mail),
    a("save.pdf_evidence", Mail),
    a("save.pdf_evidence_open", Mail),
    a("view.one_column", Mail),
    a("view.two_columns", Mail),
    a("view.three_columns", Mail),
    a("view.sort_received", Mail),
    a("view.sort_modified", Mail),
    a("view.toggle_html", Mail),
    a("view.toggle_remote_images", Mail),
    a("view.cycle_status_bar", Mail),
    a("view.reset_columns_width", Mail),
    a("view.toggle_list_layout", Mail),
    a("view.choose_theme", Mail),
    // screens
    a("screen.down", Screen),
    a("screen.up", Screen),
    a("screen.open", Screen),
    a("screen.half_page_down", Screen),
    a("screen.half_page_up", Screen),
    a("history.toggle_read", History),
    a("history.filter_source", History),
    a("contacts.filter", Contacts),
    a("contacts.decide_address", Contacts),
    a("contacts.decide_domain", Contacts),
    a("contacts.sender_kind", Contacts),
    // `tags` and `groups` share these: the scope is `screen`, and the app
    // does nothing with them on the Contacts and History screens.
    a("label.rename", Screen),
    a("label.delete", Screen),
    // panels
    a("menu.next", MenuNavigation),
    a("menu.previous", MenuNavigation),
    a("menu.choose", MenuNavigation),
    a("menu.choose_alt", MenuNavigation),
    a("picker.next", Picker),
    a("picker.previous", Picker),
    a("picker.choose", Picker),
    a("picker.choose_alt", Picker),
    a("contact_filter.close", ContactFilter),
    a("line.start", LineEdit),
    a("line.end", LineEdit),
    a("line.word_left", LineEdit),
    a("line.word_right", LineEdit),
    a("line.delete_word", LineEdit),
    a("line.delete_to_start", LineEdit),
    a("line.select_all", LineEdit),
    a("line.copy", LineEdit),
    a("line.cut", LineEdit),
    a("line.paste", LineEdit),
    a("choice.next", Choice),
    a("choice.previous", Choice),
    a("choice.choose", Choice),
    a("ai.accept", AiPanel),
    a("ai.refuse", AiPanel),
    a("ai.copy", AiPanel),
    a("ai.confirm", AiPanel),
    a("ai.open", AiPanel),
    a("ai.next", AiPanel),
    a("ai.previous", AiPanel),
    // compose
    a("compose.send", Compose),
    a("compose.file_and_send", Compose),
    a("compose.abandon", Compose),
    a("ai.compose_menu", Compose),
    a("compose.toggle_plain", Compose),
    a("compose.attach", Compose),
    a("compose.from", Compose),
    a("compose.bcc", Compose),
    a("compose.next_field", Compose),
    a("compose.previous_field", Compose),
    a("compose.paste", Compose),
    a("compose.paste_plain", Compose),
];

/// The action named `id`, if it is in the catalogue.
pub fn action(id: &str) -> Option<&'static Action> {
    ACTIONS.iter().find(|a| a.id == id)
}

/// The scope of the opener of a menu of the user, `menu.<name>`: any
/// screen. It is not in [`ACTIONS`], whose names are fixed.
pub const USER_MENU_SCOPE: Context = Context::Normal;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn names_are_unique_and_read_back() {
        let ids: BTreeSet<&str> = ACTIONS.iter().map(|a| a.id).collect();
        assert_eq!(ids.len(), ACTIONS.len(), "an action is listed twice");
        for c in Context::ALL {
            assert_eq!(Context::from_name(c.name()), Some(c));
        }
    }

    #[test]
    fn every_context_reaches_everywhere() {
        for c in Context::ALL {
            assert!(c.within(Context::Everywhere), "{c:?}");
        }
        assert!(Context::Quarantine.within(Context::Normal));
        assert!(Context::Tags.within(Context::Screen));
        assert!(!Context::Screen.within(Context::Mail));
        assert!(!Context::Compose.within(Context::Normal));
        assert!(Context::Picker.within(Context::LineEdit));
    }

    #[test]
    fn menu_names() {
        assert_eq!(menu_name("menu.save"), Some("save"));
        assert_eq!(menu_name("menu.next"), None);
        assert_eq!(menu_name("menu.choose_alt"), None);
        assert_eq!(menu_name("mail.archive"), None);
        assert!(!action("menu.save").unwrap().in_menu());
        assert!(action("save.pdf").unwrap().in_menu());
        assert!(action("goto.inbox").unwrap().in_menu());
        assert!(!action("picker.next").unwrap().in_menu());
        assert!(valid_menu_name("mine_2"));
        assert!(!valid_menu_name("Mine"));
        assert!(!valid_menu_name(""));
    }
}
